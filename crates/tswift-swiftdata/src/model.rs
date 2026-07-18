//! The Swift-facing SwiftData core surface: `@Model`, `ModelContainer`,
//! `ModelConfiguration`, and `ModelContext` (`insert`/`delete`/`save`),
//! implemented natively over the [`crate::db`] `tswift.db.*` host-service wire.
//!
//! ## What is modelled
//!
//! - **`@Model` classes.** No macro expansion: the attribute is discovered
//!   generically from the user's class declaration via
//!   [`tswift_core::StdContext::nominal_type_info`] at the point
//!   `ModelContainer(for:)` names the type. The class's stored properties
//!   become the table's columns (see [`derive_schema`] for the SQLite type
//!   mapping); the implicit SQLite `rowid` is the primary key and *is* the
//!   object's persistent identifier.
//! - **`ModelContainer(for: T.self, …)`** opens the database (creating it if
//!   absent) and runs `CREATE TABLE IF NOT EXISTS` for every model type, one
//!   table per class. Multiple model types are accepted (variadic `for:` plus
//!   any further metatype arguments). A `ModelConfiguration(isStoredInMemoryOnly:
//!   true)` argument selects an in-memory store (`":memory:"`); otherwise the
//!   store name is the configuration's `name` (defaulting to `"default.store"`)
//!   — the *host* decides what that name means on disk (a file, a `localStorage`
//!   slot, a sandbox path). It `throws` (catchable) when the platform does not
//!   back [`tswift_core::HostService::Database`].
//! - **`container.mainContext`** — the container's stable main
//!   [`ModelContext`]. **`ModelContext(container)`** creates an additional
//!   context sharing the container's connection and schema.
//! - **`context.insert(_:)` / `context.delete(_:)` / `context.save()`** track
//!   inserted / deleted / dirtied objects and, on `save()` (which `throws`),
//!   flush them as `INSERT` / `UPDATE` / `DELETE` statements inside one
//!   `begin`/`commit` transaction, rolling back on any error. Re-inserting an
//!   already-tracked object is idempotent, matching SwiftData.
//!
//! ## Deviations from real SwiftData (documented, not accidental)
//!
//! - **Autosave is OFF.** Real SwiftData's `mainContext` autosaves on run-loop
//!   ticks; this runtime has no run loop to hang autosave off, so callers must
//!   call `save()` explicitly. (`autosaveEnabled` is not modelled.)
//! - **Column types.** Only stage-1 codec property types are supported:
//!   `Int`/`Bool` → `INTEGER`, `Double` → `REAL`, `String` → `TEXT`.
//!   Non-optional properties get `NOT NULL`; `T?` allows `NULL`. `Data` and
//!   `Date` columns are **deferred** — this runtime has no primitive
//!   `SwiftValue` for either, so they would require Foundation coupling; a
//!   `@Model` declaring one raises a clear error rather than silently dropping
//!   the column.
//! - **`persistentModelID`** is the backing `rowid` and is tracked internally
//!   for identity/idempotency; a Swift-visible `.persistentModelID` accessor on
//!   the model instance is deferred (member access on a user class routes
//!   through its `ClassDef`, not this crate's builtin dispatch).
//! - **`fetch` / `#Predicate` / relationships** are out of scope for this
//!   slice (a clean seam is left: the schema + open connection already exist).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use tswift_core::{
    Arg, BuiltinReceiver, ClassObj, EvalError, Interpreter, MethodEntry, Outcome, StdContext,
    StdError, StdResult, StructObj, SwiftValue,
};
use tswift_frontend::{Node, NodeKind};

use crate::db::{self, decode_rows, encode_params, DbRow, DbValue, ExecResult};

/// The default persistent store name handed to the host when no in-memory
/// configuration is supplied. The host maps it to a real location.
const DEFAULT_STORE: &str = "default.store";

/// Hidden field the `.modelContainer(for:)` modifier stashes its `ModelContext`
/// on (on the modified view). The generic render-scope hooks read it to publish
/// the context for exactly that view's subtree. Leading `__` — never serialized
/// to the UIIR, invisible to user code, like SwiftUI's own `_env`.
const MODEL_CONTEXT_FIELD: &str = "__modelContext";

// ---------------------------------------------------------------------------
// Per-interpreter native state
//
// SwiftData containers/contexts carry Rust-native state (a db handle, the
// derived schema, and change-tracking sets holding live `Rc<RefCell<ClassObj>>`
// references) that cannot live inside a `SwiftValue`. It is held in a
// thread-local registry keyed by a small integer id that the Swift-facing
// `ModelContainer`/`ModelContext` objects carry as a hidden field.
//
// The registry is a two-level map: the outer key is the owning interpreter's
// process-unique identity ([`StdContext::interpreter_id`]), the inner key the
// per-container/context integer id. Scoping by interpreter id is what makes it
// safe for several interpreters to share one thread (e.g. concurrent live
// SwiftUI sessions): tearing one interpreter down removes only *its* bucket,
// leaving every other interpreter's containers and open handles intact. Within
// a single interpreter, the same-thread assumption of ADR-0005 still holds
// (same pattern as core's `http.rs` pending-map and `BuiltinReceiver`'s
// extension registry).
// ---------------------------------------------------------------------------

thread_local! {
    static REGISTRY: RefCell<HashMap<u64, SwiftDataState>> = RefCell::new(HashMap::new());
}

/// Run `f` against the [`SwiftDataState`] bucket owned by interpreter `iid`,
/// creating an empty bucket on first use.
fn with_state<R>(iid: u64, f: impl FnOnce(&mut SwiftDataState) -> R) -> R {
    REGISTRY.with(|r| f(r.borrow_mut().entry(iid).or_default()))
}

#[derive(Default)]
struct SwiftDataState {
    next_id: i64,
    containers: HashMap<i64, ContainerState>,
    contexts: HashMap<i64, ContextState>,
    /// The model context published to the SwiftUI environment for the *current*
    /// render subtree, read by `@Query` (via `__tswiftCurrentModelContext()`).
    /// Managed with stack discipline by the render-scope hooks
    /// ([`scope_enter`]/[`scope_exit`]): a `.modelContainer(for:)` stashes its
    /// context on the modified view (field [`MODEL_CONTEXT_FIELD`]); on entering
    /// that view's subtree the hook pushes the previous value and installs the
    /// modifier's, restoring it on exit. So nearest-ancestor wins and no context
    /// leaks across siblings or after a modifier is removed. `None` outside any
    /// `.modelContainer(for:)` subtree.
    current_context: Option<SwiftValue>,
    /// Save/restore stack backing [`current_context`]'s subtree scoping. Each
    /// custom `View` the renderer expands pushes one frame on `scope_enter` and
    /// pops it on `scope_exit`; the renderer guarantees balanced enter/exit. A
    /// non-empty stack also signals "a render is in progress", so out-of-render
    /// reads (a `Button` action firing between renders) can fall back to
    /// [`action_context`] without corrupting in-render `@Query` scoping.
    context_scope_stack: Vec<Option<SwiftValue>>,
    /// The most-recently-entered `.modelContainer(for:)` context of the last
    /// render, retained *past* the render so a `Button` action (which runs
    /// outside any render scope, where the stack is empty) can still reach a
    /// context. Single-container apps resolve unambiguously; with several
    /// containers this is last-entered (a documented limitation — the precise
    /// fix is `@Environment(\.modelContext)` capture, deferred per ADR-0016).
    /// Never consulted while a render is in progress, so it cannot leak into an
    /// unrelated subtree's `@Query`.
    ///
    /// Rebuilt from scratch every render pass (see [`pass_action_context`]): it
    /// is republished only from containers actually entered *this* pass, so a
    /// render whose `.modelContainer(for:)` disappeared (a conditional view)
    /// clears it — an out-of-render action then gets the clean no-container
    /// diagnostic instead of writing through a removed/sibling container.
    action_context: Option<SwiftValue>,
    /// Accumulator for [`action_context`], scoped to the in-flight render pass.
    /// Reset to `None` when the outermost `scope_enter` opens a pass (empty
    /// stack), overwritten by each container entered during the pass
    /// (last-entered wins), and committed to [`action_context`] when the
    /// outermost `scope_exit` closes the pass (stack returns to empty). This is
    /// what makes a stale container from a previous pass fall away instead of
    /// surviving into later out-of-render action reads.
    pass_action_context: Option<SwiftValue>,
    /// Containers minted by `.modelContainer(for:)`, keyed so the modifier
    /// reuses one per site across renders instead of re-opening the database
    /// every `body` re-evaluation. Persistent named stores share by store name +
    /// schema (same file = same database). In-memory containers are keyed by the
    /// modified view's type (the nearest stable callsite proxy available without
    /// a callsite node id) so distinct `.modelContainer(for:inMemory:)` sites do
    /// not collapse onto one shared in-memory database. Value is the container's
    /// `mainContext`. See [`env_container_key`].
    env_containers: HashMap<String, SwiftValue>,
}

struct ContainerState {
    schemas: Rc<Vec<ModelSchema>>,
    /// The stable `ModelContext` value returned by `container.mainContext`.
    main_context: SwiftValue,
}

struct ContextState {
    handle: i64,
    schemas: Rc<Vec<ModelSchema>>,
    /// Newly inserted, not-yet-flushed objects (insertion order preserved).
    inserted: Vec<Rc<RefCell<ClassObj>>>,
    /// Objects already persisted this context knows about, with their rowid and
    /// a snapshot of their column values at last flush (for dirty detection).
    tracked: Vec<Tracked>,
    /// Persisted objects marked for deletion, with the rowid to delete.
    deleted: Vec<(Rc<RefCell<ClassObj>>, i64)>,
    /// Identity map: committed persisted objects keyed by `(type name, rowid)`,
    /// so fetching the same row twice in one context returns the *same* instance
    /// (mirrors SwiftData's per-context identity map). The type name is part of
    /// the key because `rowid` is only unique *within a table*: two `@Model`
    /// types sharing one connection can each own a row with `rowid == 1`.
    /// O(1) lookup on fetch.
    by_identity: HashMap<(String, i64), Rc<RefCell<ClassObj>>>,
}

struct Tracked {
    obj: Rc<RefCell<ClassObj>>,
    rowid: i64,
    snapshot: Vec<DbValue>,
}

/// A derived table schema for one `@Model` class.
struct ModelSchema {
    type_name: String,
    table: String,
    columns: Vec<Column>,
}

struct Column {
    name: String,
    sql_type: SqlType,
    not_null: bool,
    /// Whether the Swift property is declared `Bool` (stored as `INTEGER`);
    /// needed to reconstruct a `SwiftValue::Bool` — not a plain `Int` — when
    /// decoding a fetched row.
    is_bool: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SqlType {
    Integer,
    Real,
    Text,
}

impl SqlType {
    fn sql(self) -> &'static str {
        match self {
            SqlType::Integer => "INTEGER",
            SqlType::Real => "REAL",
            SqlType::Text => "TEXT",
        }
    }
}

fn next_id(iid: u64) -> i64 {
    with_state(iid, |s| {
        s.next_id += 1;
        s.next_id
    })
}

/// Interpreter-teardown finalizer: close every open database handle *this
/// interpreter's* registry bucket holds and drop the bucket, so a session's
/// native resources are released deterministically instead of leaking. Best-
/// effort — a `tswift.db.close` that fails (e.g. an already-closed or
/// host-unbacked handle) is ignored, since teardown must never raise. Removes
/// only the calling interpreter's bucket (keyed by
/// [`StdContext::interpreter_id`]), leaving every other interpreter sharing
/// the thread untouched; contexts of one container share a handle, so each
/// unique handle is closed exactly once.
fn teardown_registry(ctx: &mut dyn StdContext) {
    let iid = ctx.interpreter_id();
    let handles: Vec<i64> = REGISTRY.with(|r| {
        let Some(state) = r.borrow_mut().remove(&iid) else {
            return Vec::new();
        };
        let mut seen = std::collections::HashSet::new();
        state
            .contexts
            .values()
            .map(|c| c.handle)
            .filter(|h| seen.insert(*h))
            .collect()
    });
    for handle in handles {
        let _ = ctx.call_host_fn(
            db::OP_CLOSE,
            vec![(Some("handle".to_string()), SwiftValue::int(handle as i128))],
        );
    }
}

// ---------------------------------------------------------------------------
// install
// ---------------------------------------------------------------------------

/// Register the SwiftData Swift-facing surface (`ModelContainer`,
/// `ModelConfiguration`, `ModelContext`) on `interp`. Always registered so the
/// initializer can raise a clean capability diagnostic when the database
/// service is unavailable; the gate is enforced at call time via
/// [`StdContext::is_host_fn`].
pub(crate) fn install(interp: &mut Interpreter<'_>) {
    // Release native state (open database handles + the change-tracking
    // registry) deterministically when the interpreter is torn down, rather
    // than leaking a handle + registry entries per container. Registered once
    // at install via the generic core finalizer seam; core knows nothing of
    // SwiftData.
    interp.register_finalizer(Box::new(teardown_registry));

    interp.register_free_fn("ModelContainer", model_container_init);
    interp.register_free_fn("ModelConfiguration", model_configuration_init);
    interp.register_free_fn("ModelContext", model_context_init);
    interp.register_free_fn("FetchDescriptor", fetch_descriptor_init);
    // `.forward`/`.reverse` resolve against the `order:` parameter's type.
    interp.register_builtin_enum("SortOrder", &["forward", "reverse"]);
    interp.register_free_fn_typed(
        "SortDescriptor",
        sort_descriptor_init,
        vec![
            tswift_core::BuiltinParam::positional("KeyPath"),
            tswift_core::BuiltinParam::labeled("order", "SortOrder"),
        ],
    );

    // `#Predicate<T> { obj in … }` — compiled to a SQL `WHERE` fragment at
    // creation time (captures resolved eagerly), via the generic macro seam.
    interp.register_macro("Predicate", predicate_macro);

    // SwiftUI integration (ADR-0016 Slice 10b). `@Query` (declared in
    // `crate::QUERY_PRELUDE`) reads the environment's model context through
    // this free fn; `.modelContainer(for:)` publishes it. Both plug into
    // `tswift-swiftui` only through generic core seams (`register_free_fn`,
    // `register_struct_method`) — SwiftUI never learns SwiftData exists.
    interp.register_free_fn("__tswiftCurrentModelContext", current_model_context);
    interp.register_struct_method("modelContainer", modifier_model_container);
    // Subtree scoping for the published context: the renderer brackets every
    // custom View's `body` with these hooks, so `.modelContainer(for:)`'s
    // context is visible only inside the modified view's subtree (nearest
    // ancestor wins) and restored afterwards — no leakage across siblings or
    // after the modifier is removed. Generic seam; SwiftUI stays SwiftData-blind.
    interp.register_view_scope(scope_enter, scope_exit);

    let container = BuiltinReceiver::register_extension("ModelContainer");
    interp.register_contextual_property(container, "mainContext", container_main_context);
    interp.register_contextual_property(container, "schema", container_schema);
    interp.register_intrinsic(
        container,
        "deleteAllData",
        MethodEntry {
            mutating: false,
            func: container_delete_all_data,
        },
    );

    let context = BuiltinReceiver::register_extension("ModelContext");
    for (name, func) in [
        ("insert", context_insert as _),
        ("delete", context_delete as _),
        ("save", context_save as _),
        ("fetch", context_fetch as _),
        ("fetchCount", context_fetch_count as _),
        ("rollback", context_rollback as _),
        ("transaction", context_transaction as _),
    ] {
        interp.register_intrinsic(
            context,
            name,
            MethodEntry {
                mutating: false,
                func,
            },
        );
    }
    interp.register_contextual_property(context, "hasChanges", context_has_changes);
    interp.register_contextual_property(context, "insertedModelsArray", context_inserted_models);
    interp.register_contextual_property(context, "changedModelsArray", context_changed_models);
    interp.register_contextual_property(context, "deletedModelsArray", context_deleted_models);

    // ModelConfiguration value-type properties: faithfully return the values
    // captured at init.
    let config = BuiltinReceiver::register_extension("ModelConfiguration");
    interp.register_contextual_property(config, "isStoredInMemoryOnly", |_ctx, recv| {
        config_property(&recv, "isStoredInMemoryOnly")
    });
    interp
        .register_contextual_property(config, "name", |_ctx, recv| config_property(&recv, "name"));

    // FetchDescriptor value-type properties: fetchLimit/fetchOffset/sortBy/
    // predicate, each reflecting the descriptor's configured state.
    let descriptor = BuiltinReceiver::register_extension("FetchDescriptor");
    interp.register_contextual_property(descriptor, "fetchLimit", |_ctx, recv| {
        fetch_descriptor_property(&recv, "fetchLimit")
    });
    interp.register_contextual_property(descriptor, "fetchOffset", |_ctx, recv| {
        fetch_descriptor_property(&recv, "fetchOffset")
    });
    interp.register_contextual_property(descriptor, "sortBy", |_ctx, recv| {
        fetch_descriptor_property(&recv, "sortBy")
    });
    interp.register_contextual_property(descriptor, "predicate", |_ctx, recv| {
        fetch_descriptor_property(&recv, "predicate")
    });
}

// ---------------------------------------------------------------------------
// Value helpers
// ---------------------------------------------------------------------------

fn make_object(class_name: &str, fields: Vec<(String, SwiftValue)>) -> SwiftValue {
    SwiftValue::Object(Rc::new(RefCell::new(ClassObj {
        class_name: class_name.to_string(),
        fields,
    })))
}

fn type_error(message: impl Into<String>) -> StdError {
    StdError::Error(EvalError::Type(message.into()))
}

/// A catchable Swift error (`HostError { message }`) — the same shape
/// `Interpreter::call_host_fn` synthesizes for a `$thrown` host reply, so a
/// SwiftData failure this crate detects is caught by `catch let e as HostError`
/// exactly like a host-signalled one.
fn host_error(message: impl Into<String>) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "HostError".into(),
        fields: vec![("message".into(), SwiftValue::Str(message.into()))],
    }))
}

fn object_int_field(value: &SwiftValue, name: &str) -> Option<i64> {
    let SwiftValue::Object(obj) = value else {
        return None;
    };
    match obj.borrow().get(name) {
        Some(SwiftValue::Int(i)) => i64::try_from(i.raw).ok(),
        _ => None,
    }
}

fn as_string(value: &SwiftValue) -> Option<String> {
    match value {
        SwiftValue::Str(s) => Some(s.clone()),
        SwiftValue::Substring { base, start, end } => Some(base[*start..*end].to_string()),
        _ => None,
    }
}

/// Read a (cloned) field from a struct-shaped view value — used by the render
/// scope hooks to recover the `ModelContext` a `.modelContainer(for:)` stashed
/// on the view it modified.
fn struct_field(value: &SwiftValue, name: &str) -> Option<SwiftValue> {
    match value {
        SwiftValue::Struct(obj) => obj.get(name).cloned(),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Schema derivation
// ---------------------------------------------------------------------------

/// Split a declared type spelling into `(base, is_optional)`, stripping a
/// single trailing `?` (`"Int?"` → `("Int", true)`). Nested optionals and
/// generic wrappers beyond a bare `T?` are not modelled.
fn split_optional(ty: &str) -> (&str, bool) {
    let trimmed = ty.trim();
    if let Some(base) = trimmed.strip_suffix('?') {
        (base.trim(), true)
    } else {
        (trimmed, false)
    }
}

fn sql_type_for(base: &str) -> Option<SqlType> {
    match base {
        "Int" | "Int64" | "Int32" | "Int16" | "Int8" | "UInt" | "UInt64" | "UInt32" | "UInt16"
        | "UInt8" | "Bool" => Some(SqlType::Integer),
        "Double" | "Float" | "Float64" | "Float32" | "CGFloat" => Some(SqlType::Real),
        "String" => Some(SqlType::Text),
        _ => None,
    }
}

/// Derive a table schema from a `@Model` class named `type_name`, or a
/// catchable error describing why it cannot be modelled.
fn derive_schema(ctx: &dyn StdContext, type_name: &str) -> Result<ModelSchema, SwiftValue> {
    let Some(info) = ctx.nominal_type_info(type_name) else {
        return Err(host_error(format!(
            "SwiftData: no class named '{type_name}' is declared for this ModelContainer"
        )));
    };
    if !info.attributes.iter().any(|a| a == "Model") {
        return Err(host_error(format!(
            "SwiftData: '{type_name}' is not a @Model class"
        )));
    }
    let mut columns = Vec::with_capacity(info.stored.len());
    for prop in &info.stored {
        let Some(declared) = &prop.declared_type else {
            return Err(host_error(format!(
                "SwiftData: property '{type_name}.{}' has no explicit type; @Model stored properties must be annotated",
                prop.name
            )));
        };
        let (base, optional) = split_optional(declared);
        match sql_type_for(base) {
            Some(sql_type) => columns.push(Column {
                name: prop.name.clone(),
                sql_type,
                not_null: !optional,
                is_bool: base == "Bool",
            }),
            None => {
                return Err(host_error(format!(
                    "SwiftData: property '{type_name}.{}' has unsupported type '{declared}' (supported: Int, Double, String, Bool and their optionals; Data/Date are not yet supported)",
                    prop.name
                )))
            }
        }
    }
    if columns.is_empty() {
        return Err(host_error(format!(
            "SwiftData: @Model class '{type_name}' has no stored properties to persist"
        )));
    }
    Ok(ModelSchema {
        type_name: type_name.to_string(),
        table: type_name.to_string(),
        columns,
    })
}

/// Quote a SQL identifier by wrapping it in double quotes and doubling any
/// embedded quote. Type/property names are Swift identifiers (no quotes), but
/// quoting defensively keeps generated SQL well-formed regardless.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn create_table_sql(schema: &ModelSchema) -> String {
    let cols: Vec<String> = schema
        .columns
        .iter()
        .map(|c| {
            let null = if c.not_null { " NOT NULL" } else { "" };
            format!("{} {}{}", quote_ident(&c.name), c.sql_type.sql(), null)
        })
        .collect();
    format!(
        "CREATE TABLE IF NOT EXISTS {} ({})",
        quote_ident(&schema.table),
        cols.join(", ")
    )
}

// ---------------------------------------------------------------------------
// ModelConfiguration(...)
// ---------------------------------------------------------------------------

/// `ModelConfiguration(isStoredInMemoryOnly:)` / `ModelConfiguration(_ name,
/// isStoredInMemoryOnly:)`. Represented as an opaque object carrying the two
/// fields the container reads.
fn model_configuration_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut in_memory = false;
    let mut name: Option<String> = None;
    for arg in &args {
        match arg.label.as_deref() {
            Some("isStoredInMemoryOnly") => {
                if let SwiftValue::Bool(b) = arg.value {
                    in_memory = b;
                }
            }
            // The leading unlabeled positional is the configuration name.
            None => {
                if let Some(s) = as_string(&arg.value) {
                    name = Some(s);
                }
            }
            _ => {}
        }
    }
    Ok(make_object(
        "ModelConfiguration",
        vec![
            ("isStoredInMemoryOnly".into(), SwiftValue::Bool(in_memory)),
            (
                "name".into(),
                name.map(SwiftValue::Str).unwrap_or(SwiftValue::Nil),
            ),
        ],
    ))
}

fn configuration_fields(value: &SwiftValue) -> Option<(bool, Option<String>)> {
    let SwiftValue::Object(obj) = value else {
        return None;
    };
    let obj = obj.borrow();
    if obj.class_name != "ModelConfiguration" {
        return None;
    }
    let in_memory = matches!(
        obj.get("isStoredInMemoryOnly"),
        Some(SwiftValue::Bool(true))
    );
    let name = obj.get("name").and_then(as_string);
    Some((in_memory, name))
}

// ---------------------------------------------------------------------------
// ModelContainer(for: T.self, …)
// ---------------------------------------------------------------------------

/// The parsed model-type set and store selection of a `ModelContainer(for:)` /
/// `.modelContainer(for:)` argument list.
struct ContainerArgs {
    type_names: Vec<String>,
    in_memory: bool,
    store_name: Option<String>,
}

/// Collect every model type (any metatype argument, in any position — the
/// `for:` label leads the variadic) and any `ModelConfiguration`. Shared by the
/// `ModelContainer(for:)` initializer and the `.modelContainer(for:)` modifier.
fn parse_container_args(args: &[Arg]) -> ContainerArgs {
    let mut type_names: Vec<String> = Vec::new();
    let mut in_memory = false;
    let mut store_name: Option<String> = None;
    let mut push_type = |v: &SwiftValue| {
        if let SwiftValue::Metatype(name) = v {
            if !type_names.contains(name) {
                type_names.push(name.clone());
            }
        }
    };
    for arg in args {
        // `.modelContainer(for:inMemory:)` — the modifier's convenience Bool
        // (distinct from a ModelConfiguration). Any other label is ignored.
        if arg.label.as_deref() == Some("inMemory") {
            if let SwiftValue::Bool(b) = arg.value {
                in_memory |= b;
            }
            continue;
        }
        match &arg.value {
            SwiftValue::Metatype(_) => push_type(&arg.value),
            SwiftValue::Array(items) => {
                for item in items.iter() {
                    if let Some((mem, name)) = configuration_fields(item) {
                        in_memory |= mem;
                        store_name = store_name.or(name);
                    } else {
                        push_type(item);
                    }
                }
            }
            _ => {
                if let Some((mem, name)) = configuration_fields(&arg.value) {
                    in_memory |= mem;
                    store_name = store_name.or(name);
                }
            }
        }
    }
    ContainerArgs {
        type_names,
        in_memory,
        store_name,
    }
}

/// Derive schemas, open the store, create the tables, and register a fresh
/// container + its stable `mainContext` in the registry. Returns the container
/// value paired with its main-context value. Shared by the initializer and the
/// environment modifier; performs the capability gate itself.
fn open_container(ctx: &mut dyn StdContext, parsed: &ContainerArgs) -> StdResult {
    let ContainerArgs {
        type_names,
        in_memory,
        store_name,
    } = parsed;

    if type_names.is_empty() {
        return Err(type_error(
            "ModelContainer(for:) requires at least one model type (e.g. ModelContainer(for: Movie.self))",
        ));
    }

    // Capability gate: a clean, catchable diagnostic when no database backing.
    if !ctx.is_host_fn(db::OP_OPEN) {
        return Err(ctx.throw(host_error(
            "SwiftData is unavailable on this platform: the host does not provide the 'tswift.db' service",
        )));
    }

    // Derive schemas before opening anything, so a bad @Model fails cleanly.
    let mut schemas = Vec::with_capacity(type_names.len());
    for name in type_names {
        match derive_schema(ctx, name) {
            Ok(schema) => schemas.push(schema),
            Err(err) => return Err(ctx.throw(err)),
        }
    }
    let schemas = Rc::new(schemas);

    let path = if *in_memory {
        ":memory:".to_string()
    } else {
        store_name
            .clone()
            .unwrap_or_else(|| DEFAULT_STORE.to_string())
    };

    // Open the database (creating it if absent).
    let handle_val = ctx.call_host_fn(
        db::OP_OPEN,
        vec![(Some("path".to_string()), SwiftValue::Str(path))],
    )?;
    let handle = match handle_val {
        SwiftValue::Int(i) => i64::try_from(i.raw)
            .map_err(|_| type_error("SwiftData: host returned an out-of-range db handle"))?,
        other => {
            return Err(type_error(format!(
                "SwiftData: tswift.db.open returned {}, expected Int",
                other.type_name()
            )))
        }
    };

    // Create one table per model type (no per-property round-trips).
    for schema in schemas.iter() {
        execute(ctx, handle, &create_table_sql(schema), &[])?;
    }

    // Build the container's stable main context, then the container itself.
    let iid = ctx.interpreter_id();
    let main_ctx_id = next_id(iid);
    let main_context = make_object(
        "ModelContext",
        vec![("__ctxid".into(), SwiftValue::int(main_ctx_id as i128))],
    );
    let cid = next_id(iid);
    with_state(iid, |s| {
        s.contexts.insert(
            main_ctx_id,
            ContextState {
                handle,
                schemas: Rc::clone(&schemas),
                inserted: Vec::new(),
                tracked: Vec::new(),
                deleted: Vec::new(),
                by_identity: HashMap::new(),
            },
        );
        s.containers.insert(
            cid,
            ContainerState {
                schemas,
                main_context: main_context.clone(),
            },
        );
    });

    Ok(make_object(
        "ModelContainer",
        vec![("__cid".into(), SwiftValue::int(cid as i128))],
    ))
}

fn model_container_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let parsed = parse_container_args(&args);
    open_container(ctx, &parsed)
}

/// The main-context value of a container value (`__cid` → registry lookup).
fn container_main_context_value(iid: u64, container: &SwiftValue) -> Option<SwiftValue> {
    let cid = object_int_field(container, "__cid")?;
    with_state(iid, |s| {
        s.containers.get(&cid).map(|c| c.main_context.clone())
    })
}

/// The reuse key for a `.modelContainer(for:)` site's container. Persistent
/// named stores share by store name + schema (same file = same database, which
/// SwiftData also does). In-memory containers are per *site*: two distinct
/// `.modelContainer(for:inMemory:)` with the same schema must own separate
/// databases (each modifier instance owns its container). Lacking a callsite
/// node id in the struct-method seam, the modified view's type name is the
/// nearest stable site proxy — sibling containers on distinct views stay
/// isolated. (Tripwire: two in-memory containers on the *same* view type would
/// still collide; revisit if the renderer ever exposes a real callsite id.)
fn env_container_key(parsed: &ContainerArgs, recv: &SwiftValue) -> String {
    let mut sorted = parsed.type_names.clone();
    sorted.sort();
    let schemas = sorted.join(",");
    if parsed.in_memory {
        format!("mem|{}|{schemas}", recv.type_name())
    } else {
        // Normalize the store name to what `open_container` actually opens: an
        // absent or empty name maps to `DEFAULT_STORE`. Without this an explicit
        // `ModelConfiguration("default.store")` would key differently from the
        // implicit default and mint a *second* container over the same file.
        let store = match parsed.store_name.as_deref() {
            Some(name) if !name.is_empty() => name,
            _ => DEFAULT_STORE,
        };
        format!("persist|{store}|{schemas}")
    }
}

/// `.modelContainer(for: T.self)` / `.modelContainer(for: [A.self, B.self])` —
/// the SwiftUI view modifier. Establishes (once per site, then reused across
/// renders) a container and stashes its `mainContext` on the modified view
/// (field [`MODEL_CONTEXT_FIELD`]). The render-scope hooks ([`scope_enter`]/
/// [`scope_exit`]) publish it to the environment for exactly that view's
/// subtree, so `@Query` reads the nearest-ancestor context with no leakage. A
/// side-effecting passthrough on the render tree: the receiver view is returned
/// (carrying the stashed field) and never reaches the UIIR, like
/// `.environmentObject(_)`.
///
/// Registered from this crate via the generic `Interpreter::register_struct_method`
/// seam — `tswift-swiftui` never learns SwiftData exists.
fn modifier_model_container(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> StdResult {
    let parsed = parse_container_args(&args);
    if parsed.type_names.is_empty() {
        return Err(type_error(
            ".modelContainer(for:) requires at least one model type (e.g. .modelContainer(for: Item.self))",
        ));
    }
    let key = env_container_key(&parsed, &recv);
    let iid = ctx.interpreter_id();
    let existing = with_state(iid, |s| s.env_containers.get(&key).cloned());
    let main_context = match existing {
        Some(mc) => mc,
        None => {
            let container = open_container(ctx, &parsed)?;
            let mc = container_main_context_value(iid, &container).ok_or_else(|| {
                type_error(".modelContainer(for:): failed to establish a main context")
            })?;
            with_state(iid, |s| {
                s.env_containers.insert(key, mc.clone());
            });
            mc
        }
    };
    // Stash the context on the modified view; the render-scope hooks publish it
    // for this subtree only. Do NOT set `current_context` here — that would leak
    // to siblings evaluated later in the same render.
    let SwiftValue::Struct(obj) = &recv else {
        return Err(type_error(format!(
            ".modelContainer(for:) applied to non-view value `{}`",
            recv.type_name()
        )));
    };
    let mut fields = obj.fields.clone();
    fields.retain(|(k, _)| k != MODEL_CONTEXT_FIELD);
    fields.push((MODEL_CONTEXT_FIELD.into(), main_context));
    Ok(SwiftValue::Struct(Rc::new(StructObj {
        type_name: obj.type_name.clone(),
        fields,
    })))
}

/// Render-scope enter hook (registered via `Interpreter::register_view_scope`).
/// Push the current published context, then — if this view was modified by
/// `.modelContainer(for:)` — install that modifier's context for the subtree.
/// Balanced with [`scope_exit`] by the renderer.
fn scope_enter(ctx: &mut dyn StdContext, view: &SwiftValue) {
    let marker = struct_field(view, MODEL_CONTEXT_FIELD);
    with_state(ctx.interpreter_id(), |s| {
        // Outermost enter (empty stack) opens a new render pass: forget the
        // previous pass's accumulated action context, so a container removed
        // this pass does not survive into it.
        if s.context_scope_stack.is_empty() {
            s.pass_action_context = None;
        }
        s.context_scope_stack.push(s.current_context.clone());
        if let Some(mc) = marker {
            s.current_context = Some(mc.clone());
            // Record for out-of-render action reads; committed to
            // `action_context` only when this pass ends (see `scope_exit`).
            s.pass_action_context = Some(mc);
        }
    });
}

/// Render-scope exit hook. Restore the context saved by the matching
/// [`scope_enter`], so nearest-ancestor scoping holds and nothing leaks past the
/// subtree.
fn scope_exit(ctx: &mut dyn StdContext, _view: &SwiftValue) {
    with_state(ctx.interpreter_id(), |s| {
        if let Some(prev) = s.context_scope_stack.pop() {
            s.current_context = prev;
        }
        // Outermost exit (stack back to empty) closes the render pass: publish
        // the containers actually entered this pass, clearing any stale action
        // context from a pass whose `.modelContainer(for:)` has since vanished.
        if s.context_scope_stack.is_empty() {
            s.action_context = s.pass_action_context.take();
        }
    });
}

/// `__tswiftCurrentModelContext()` — the environment's current `ModelContext`,
/// published by the nearest enclosing `.modelContainer(for:)`. Throws a
/// catchable error when no container is in scope, so `@Query`'s getter degrades
/// to an empty array (via `try?`) rather than trapping.
fn current_model_context(ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    // While a render is in progress (scope stack non-empty), honour the strict
    // subtree scope so `@Query` sees only its nearest-ancestor container. Only
    // out-of-render callers (a `Button` action firing between renders) fall back
    // to the retained `action_context`.
    let resolved = with_state(ctx.interpreter_id(), |s| {
        if s.context_scope_stack.is_empty() {
            s.action_context.clone()
        } else {
            s.current_context.clone()
        }
    });
    match resolved {
        Some(mc) => Ok(mc),
        None => Err(ctx.throw(host_error(
            "SwiftData: no model container in the environment; add .modelContainer(for:) to a view",
        ))),
    }
}

fn container_main_context(ctx: &mut dyn StdContext, recv: SwiftValue) -> StdResult {
    let Some(cid) = object_int_field(&recv, "__cid") else {
        return Err(type_error(
            "ModelContainer.mainContext: not a ModelContainer",
        ));
    };
    with_state(ctx.interpreter_id(), |s| {
        s.containers
            .get(&cid)
            .map(|c| c.main_context.clone())
            .ok_or_else(|| type_error("ModelContainer.mainContext: unknown container"))
    })
}

/// `ModelContainer.schema` — a lightweight `Schema` value carrying the entity
/// (model) type names the container was created with, in registration order.
fn container_schema(ctx: &mut dyn StdContext, recv: SwiftValue) -> StdResult {
    let Some(cid) = object_int_field(&recv, "__cid") else {
        return Err(type_error("ModelContainer.schema: not a ModelContainer"));
    };
    let entities = with_state(ctx.interpreter_id(), |s| {
        s.containers.get(&cid).map(|c| {
            c.schemas
                .iter()
                .map(|sc| SwiftValue::Str(sc.type_name.clone()))
                .collect::<Vec<_>>()
        })
    })
    .ok_or_else(|| type_error("ModelContainer.schema: unknown container"))?;
    Ok(make_object(
        "Schema",
        vec![("entityNames".into(), SwiftValue::Array(Rc::new(entities)))],
    ))
}

/// `ModelContainer.deleteAllData()` — clear every row from every table the
/// container manages, then discard the in-context tracking of every context on
/// that connection (their cached rows are now gone). Real behavior against the
/// store: one `DELETE FROM <table>` per model type.
fn container_delete_all_data(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let cid = object_int_field(&recv, "__cid")
        .ok_or_else(|| type_error("ModelContainer.deleteAllData(): not a ModelContainer"))?;
    let iid = ctx.interpreter_id();
    let resolved = with_state(iid, |s| {
        let container = s.containers.get(&cid)?;
        let main_ctx_id = object_int_field(&container.main_context, "__ctxid")?;
        let handle = s.contexts.get(&main_ctx_id)?.handle;
        Some((handle, Rc::clone(&container.schemas)))
    });
    let Some((handle, schemas)) = resolved else {
        return Err(type_error(
            "ModelContainer.deleteAllData(): unknown container",
        ));
    };
    for schema in schemas.iter() {
        execute(
            ctx,
            handle,
            &format!("DELETE FROM {}", quote_ident(&schema.table)),
            &[],
        )?;
    }
    // Every context on this connection has now-stale caches; reset their
    // tracking so subsequent fetches see the emptied store.
    with_state(iid, |s| {
        for state in s.contexts.values_mut() {
            if state.handle == handle {
                state.inserted.clear();
                state.tracked.clear();
                state.deleted.clear();
                state.by_identity.clear();
            }
        }
    });
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

// ---------------------------------------------------------------------------
// ModelContext(container)
// ---------------------------------------------------------------------------

fn model_context_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let container = args
        .iter()
        .find(|a| matches!(&a.value, SwiftValue::Object(o) if o.borrow().class_name == "ModelContainer"))
        .map(|a| a.value.clone());
    let Some(container) = container else {
        return Err(type_error(
            "ModelContext(_:) expects a ModelContainer argument",
        ));
    };
    let Some(cid) = object_int_field(&container, "__cid") else {
        return Err(type_error(
            "ModelContext(_:) received an invalid ModelContainer",
        ));
    };
    let iid = ctx.interpreter_id();
    let schemas = with_state(iid, |s| {
        s.containers.get(&cid).map(|c| Rc::clone(&c.schemas))
    });
    let Some(schemas) = schemas else {
        return Err(type_error(
            "ModelContext(_:) received an unknown ModelContainer",
        ));
    };
    // A fresh context shares the main context's connection handle + schema.
    let handle = with_state(iid, |s| {
        // Every context of a container shares one handle; read it from the
        // container's main context.
        let main_cid = object_int_field(&s.containers.get(&cid).unwrap().main_context, "__ctxid")?;
        s.contexts.get(&main_cid).map(|c| c.handle)
    });
    let Some(handle) = handle else {
        return Err(type_error(
            "ModelContext(_:): container has no open connection",
        ));
    };
    let id = next_id(iid);
    with_state(iid, |s| {
        s.contexts.insert(
            id,
            ContextState {
                handle,
                schemas,
                inserted: Vec::new(),
                tracked: Vec::new(),
                deleted: Vec::new(),
                by_identity: HashMap::new(),
            },
        );
    });
    Ok(make_object(
        "ModelContext",
        vec![("__ctxid".into(), SwiftValue::int(id as i128))],
    ))
}

// ---------------------------------------------------------------------------
// insert / delete / save
// ---------------------------------------------------------------------------

fn context_id(recv: &SwiftValue) -> Result<i64, StdError> {
    object_int_field(recv, "__ctxid")
        .ok_or_else(|| type_error("ModelContext method called on a non-ModelContext value"))
}

fn model_object(args: &[SwiftValue], who: &str) -> Result<Rc<RefCell<ClassObj>>, StdError> {
    match args.first() {
        Some(SwiftValue::Object(o)) => Ok(Rc::clone(o)),
        _ => Err(type_error(format!("{who} expects a @Model class instance"))),
    }
}

fn context_insert(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let id = context_id(&recv)?;
    let obj = model_object(&args, "ModelContext.insert(_:)")?;
    with_state(ctx.interpreter_id(), |s| {
        if let Some(state) = s.contexts.get_mut(&id) {
            let known = state.inserted.iter().any(|o| Rc::ptr_eq(o, &obj))
                || state.tracked.iter().any(|t| Rc::ptr_eq(&t.obj, &obj));
            // Re-inserting a pending-delete object cancels the delete.
            state.deleted.retain(|(o, _)| !Rc::ptr_eq(o, &obj));
            if !known {
                state.inserted.push(obj);
            }
        }
    });
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

fn context_delete(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let id = context_id(&recv)?;
    let obj = model_object(&args, "ModelContext.delete(_:)")?;
    with_state(ctx.interpreter_id(), |s| {
        if let Some(state) = s.contexts.get_mut(&id) {
            // Never-persisted (still pending insert): just drop it.
            let before = state.inserted.len();
            state.inserted.retain(|o| !Rc::ptr_eq(o, &obj));
            if state.inserted.len() != before {
                return;
            }
            // Persisted: move to the delete set (dedup by identity).
            if let Some(pos) = state.tracked.iter().position(|t| Rc::ptr_eq(&t.obj, &obj)) {
                let tracked = state.tracked.remove(pos);
                if !state.deleted.iter().any(|(o, _)| Rc::ptr_eq(o, &obj)) {
                    state.deleted.push((tracked.obj, tracked.rowid));
                }
            }
        }
    });
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

fn context_save(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let id = context_id(&recv)?;
    let iid = ctx.interpreter_id();
    // Take the context state out so host calls (which need `ctx`) don't alias
    // the thread-local borrow; always put it back, mutated only on success.
    let mut state = with_state(iid, |s| s.contexts.remove(&id))
        .ok_or_else(|| type_error("ModelContext.save(): unknown context"))?;
    let result = flush(ctx, &mut state);
    with_state(iid, |s| {
        s.contexts.insert(id, state);
    });
    result?;
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

/// Flush a context's pending changes inside one transaction. On error, rolls
/// back and leaves `state`'s tracking sets unchanged; on success, applies the
/// inserts (moving them into `tracked` with their new rowids), refreshes dirty
/// snapshots, and clears the delete set.
fn flush(ctx: &mut dyn StdContext, state: &mut ContextState) -> Result<(), StdError> {
    // Compute the update plan (dirty tracked rows) up front so we can decide
    // whether there is anything to do and avoid a transaction for a no-op save.
    let mut updates: Vec<(usize, Vec<DbValue>)> = Vec::new();
    for (idx, tracked) in state.tracked.iter().enumerate() {
        let schema = schema_for(&state.schemas, &tracked.obj.borrow().class_name)?;
        let values = row_values(&tracked.obj.borrow(), schema)?;
        if values != tracked.snapshot {
            updates.push((idx, values));
        }
    }
    if state.inserted.is_empty() && updates.is_empty() && state.deleted.is_empty() {
        return Ok(());
    }

    let handle = state.handle;
    tx(ctx, handle, db::OP_BEGIN)?;

    // Run the body; on any error roll back (best-effort) and propagate.
    match flush_body(ctx, state, &updates) {
        Ok(insert_rowids) => {
            // A COMMIT can itself fail (e.g. a deferred constraint check or an
            // I/O error at commit time). If it does, the transaction is still
            // open, so roll it back (best-effort) to release it — otherwise the
            // connection is left mid-transaction and every later `save()` on
            // this handle would fail with "cannot start a transaction within a
            // transaction". Crucially, do NOT apply `apply_after_commit`: the
            // changes never landed, so the tracking sets stay untouched and the
            // dirty objects remain pending, mirroring SwiftData's `save()`
            // failure semantics (the changes are still in the context, to be
            // retried). Then surface the original commit error to the caller.
            if let Err(err) = tx(ctx, handle, db::OP_COMMIT) {
                let _ = tx(ctx, handle, db::OP_ROLLBACK);
                return Err(err);
            }
            // Apply state changes only after a successful commit.
            apply_after_commit(state, insert_rowids, updates);
            Ok(())
        }
        Err(err) => {
            let _ = tx(ctx, handle, db::OP_ROLLBACK);
            Err(err)
        }
    }
}

/// Execute the INSERT/UPDATE/DELETE statements. Returns, for each pending
/// insert (in order), its new rowid.
fn flush_body(
    ctx: &mut dyn StdContext,
    state: &ContextState,
    updates: &[(usize, Vec<DbValue>)],
) -> Result<Vec<i64>, StdError> {
    let handle = state.handle;
    let mut insert_rowids = Vec::with_capacity(state.inserted.len());
    for obj in &state.inserted {
        let schema = schema_for(&state.schemas, &obj.borrow().class_name)?;
        let values = row_values(&obj.borrow(), schema)?;
        let result = execute(ctx, handle, &insert_sql(schema), &values)?;
        insert_rowids.push(result.last_insert_rowid);
    }
    for (idx, values) in updates {
        let tracked = &state.tracked[*idx];
        let schema = schema_for(&state.schemas, &tracked.obj.borrow().class_name)?;
        let mut params = values.clone();
        params.push(DbValue::Int(tracked.rowid));
        execute(ctx, handle, &update_sql(schema), &params)?;
    }
    for (obj, rowid) in &state.deleted {
        let schema = schema_for(&state.schemas, &obj.borrow().class_name)?;
        execute(ctx, handle, &delete_sql(schema), &[DbValue::Int(*rowid)])?;
    }
    Ok(insert_rowids)
}

fn apply_after_commit(
    state: &mut ContextState,
    insert_rowids: Vec<i64>,
    updates: Vec<(usize, Vec<DbValue>)>,
) {
    // Refresh dirty snapshots for updated rows.
    for (idx, values) in updates {
        state.tracked[idx].snapshot = values;
    }
    // Move inserted objects into the tracked set with their new rowid+snapshot,
    // and register them in the identity map.
    let inserted = std::mem::take(&mut state.inserted);
    for (obj, rowid) in inserted.into_iter().zip(insert_rowids) {
        let (class_name, snapshot) = {
            let borrowed = obj.borrow();
            let snapshot = schema_for(&state.schemas, &borrowed.class_name)
                .and_then(|schema| row_values(&borrowed, schema))
                .unwrap_or_default();
            (borrowed.class_name.clone(), snapshot)
        };
        state
            .by_identity
            .insert((class_name, rowid), Rc::clone(&obj));
        state.tracked.push(Tracked {
            obj,
            rowid,
            snapshot,
        });
    }
    // Drop committed-deleted objects from the identity map (keyed by type+rowid).
    for (obj, rowid) in &state.deleted {
        let class_name = obj.borrow().class_name.clone();
        state.by_identity.remove(&(class_name, *rowid));
    }
    state.deleted.clear();
}

fn schema_for<'a>(
    schemas: &'a [ModelSchema],
    class_name: &str,
) -> Result<&'a ModelSchema, StdError> {
    schemas
        .iter()
        .find(|s| s.type_name == class_name)
        .ok_or_else(|| {
            type_error(format!(
                "SwiftData: '{class_name}' is not registered with this ModelContainer"
            ))
        })
}

/// Extract a row's column values from a model instance, in schema order.
fn row_values(obj: &ClassObj, schema: &ModelSchema) -> Result<Vec<DbValue>, StdError> {
    let mut values = Vec::with_capacity(schema.columns.len());
    for col in &schema.columns {
        let field = obj.get(&col.name).cloned().unwrap_or(SwiftValue::Nil);
        values.push(encode_field(&field, col, &schema.type_name)?);
    }
    Ok(values)
}

fn encode_field(value: &SwiftValue, col: &Column, type_name: &str) -> Result<DbValue, StdError> {
    if matches!(value, SwiftValue::Nil) {
        if col.not_null {
            return Err(type_error(format!(
                "SwiftData: non-optional property '{type_name}.{}' is nil",
                col.name
            )));
        }
        return Ok(DbValue::Null);
    }
    let mismatch = || {
        type_error(format!(
            "SwiftData: property '{type_name}.{}' has a value that does not match its {} column",
            col.name,
            col.sql_type.sql()
        ))
    };
    match col.sql_type {
        SqlType::Integer => match value {
            SwiftValue::Int(i) => i64::try_from(i.raw).map(DbValue::Int).map_err(|_| {
                type_error(format!(
                    "SwiftData: property '{type_name}.{}' value {} does not fit in Int64",
                    col.name, i.raw
                ))
            }),
            SwiftValue::Bool(b) => Ok(DbValue::Int(i64::from(*b))),
            _ => Err(mismatch()),
        },
        SqlType::Real => match value {
            SwiftValue::Double(d) => Ok(DbValue::Real(*d)),
            SwiftValue::Int(i) => Ok(DbValue::Real(i.raw as f64)),
            _ => Err(mismatch()),
        },
        SqlType::Text => match as_string(value) {
            Some(s) => Ok(DbValue::Text(s)),
            None => Err(mismatch()),
        },
    }
}

fn insert_sql(schema: &ModelSchema) -> String {
    let cols: Vec<String> = schema
        .columns
        .iter()
        .map(|c| quote_ident(&c.name))
        .collect();
    let placeholders: Vec<&str> = schema.columns.iter().map(|_| "?").collect();
    format!(
        "INSERT INTO {} ({}) VALUES ({})",
        quote_ident(&schema.table),
        cols.join(", "),
        placeholders.join(", ")
    )
}

fn update_sql(schema: &ModelSchema) -> String {
    let assignments: Vec<String> = schema
        .columns
        .iter()
        .map(|c| format!("{} = ?", quote_ident(&c.name)))
        .collect();
    format!(
        "UPDATE {} SET {} WHERE rowid = ?",
        quote_ident(&schema.table),
        assignments.join(", ")
    )
}

fn delete_sql(schema: &ModelSchema) -> String {
    format!("DELETE FROM {} WHERE rowid = ?", quote_ident(&schema.table))
}

// ---------------------------------------------------------------------------
// Host-wire helpers
// ---------------------------------------------------------------------------

fn execute(
    ctx: &mut dyn StdContext,
    handle: i64,
    sql: &str,
    params: &[DbValue],
) -> Result<ExecResult, StdError> {
    let reply = ctx.call_host_fn(
        db::OP_EXECUTE,
        vec![
            (Some("handle".to_string()), SwiftValue::int(handle as i128)),
            (Some("sql".to_string()), SwiftValue::Str(sql.to_string())),
            (
                Some("params".to_string()),
                SwiftValue::Str(encode_params(params)),
            ),
        ],
    )?;
    let text = as_string(&reply)
        .ok_or_else(|| type_error("SwiftData: tswift.db.execute returned a non-String reply"))?;
    ExecResult::decode(&text)
        .map_err(|e| type_error(format!("SwiftData: malformed execute result: {e}")))
}

fn tx(ctx: &mut dyn StdContext, handle: i64, op: &str) -> Result<(), StdError> {
    ctx.call_host_fn(
        op,
        vec![(Some("handle".to_string()), SwiftValue::int(handle as i128))],
    )?;
    Ok(())
}

/// Run a `SELECT` (or other read) via the `tswift.db.query` wire and decode the
/// reply into rows.
fn query(
    ctx: &mut dyn StdContext,
    handle: i64,
    sql: &str,
    params: &[DbValue],
) -> Result<Vec<DbRow>, StdError> {
    let reply = ctx.call_host_fn(
        db::OP_QUERY,
        vec![
            (Some("handle".to_string()), SwiftValue::int(handle as i128)),
            (Some("sql".to_string()), SwiftValue::Str(sql.to_string())),
            (
                Some("params".to_string()),
                SwiftValue::Str(encode_params(params)),
            ),
        ],
    )?;
    let text = as_string(&reply)
        .ok_or_else(|| type_error("SwiftData: tswift.db.query returned a non-String reply"))?;
    decode_rows(&text).map_err(|e| type_error(format!("SwiftData: malformed query result: {e}")))
}

// ---------------------------------------------------------------------------
// #Predicate<T> { obj in … }  →  SQL WHERE fragment (bound params)
// ---------------------------------------------------------------------------
//
// The macro handler compiles the closure body to a SQL `WHERE` fragment with
// `?` placeholders at *creation* time — resolving any captured/literal value
// eagerly to a bound parameter (mirroring SwiftData capturing values when the
// `#Predicate` is formed). The compiled predicate is stored on the returned
// opaque `Predicate` object as three fields the fetch path reads: `__where`
// (fragment text, empty for a trivially-true predicate), `__params` (the JSON
// bind array) and `__type` (the `<T>` model type, if written).
//
// Deviation from Apple SwiftData: real `#Predicate` is a compile-time macro
// evaluated lazily against each object; here it lowers straight to SQL, so
// only the shapes SQLite can express with bound params are supported. Anything
// else raises a clear diagnostic rather than silently full-scanning with a
// wrong (or absent) filter. See ADR-0016.

/// The set of comparison operators lowered directly to SQL.
fn sql_comparison_op(op: &str) -> Option<&'static str> {
    match op {
        "==" => Some("="),
        "!=" => Some("<>"),
        "<" => Some("<"),
        "<=" => Some("<="),
        ">" => Some(">"),
        ">=" => Some(">="),
        _ => None,
    }
}

/// Flip a comparison operator when its operands are swapped
/// (`2000 < obj.year` → `obj.year > 2000`).
fn flip_comparison(op: &str) -> &str {
    match op {
        "<" => ">",
        "<=" => ">=",
        ">" => "<",
        ">=" => "<=",
        other => other, // ==, != are symmetric
    }
}

fn predicate_error(msg: impl Into<String>) -> StdError {
    // A clear, non-silent diagnostic: an unsupported predicate shape must never
    // degrade into a wrong or absent filter.
    type_error(format!("SwiftData #Predicate: {}", msg.into()))
}

/// Whether `node`'s subtree references the closure parameter `param`.
fn references_param(node: &Node<'static>, param: &str) -> bool {
    if node.kind() == NodeKind::IdentExpr && node.text().as_deref() == Some(param) {
        return true;
    }
    node.children().any(|c| references_param(&c, param))
}

/// If `node` is `param.property` (a single-level member access on the closure
/// parameter), return the property/column name.
fn param_column(node: &Node<'static>, param: &str) -> Option<String> {
    if node.kind() != NodeKind::MemberExpr {
        return None;
    }
    let base = node.children().next()?;
    if base.kind() == NodeKind::IdentExpr && base.text().as_deref() == Some(param) {
        node.text()
    } else {
        None
    }
}

/// Convert an evaluated Swift value into a bound SQL value.
fn swift_to_db_value(value: &SwiftValue) -> Result<DbValue, StdError> {
    match value {
        SwiftValue::Nil => Ok(DbValue::Null),
        SwiftValue::Bool(b) => Ok(DbValue::Int(i64::from(*b))),
        SwiftValue::Int(i) => i64::try_from(i.raw)
            .map(DbValue::Int)
            .map_err(|_| predicate_error("integer literal does not fit in Int64")),
        SwiftValue::Double(d) => Ok(DbValue::Real(*d)),
        other => match as_string(other) {
            Some(s) => Ok(DbValue::Text(s)),
            None => Err(predicate_error(format!(
                "cannot bind a value of type {} as a query parameter",
                other.type_name()
            ))),
        },
    }
}

/// Escape a `LIKE` pattern literal so `%` / `_` / `\\` are matched literally
/// (the generated `LIKE` clause pairs this with `ESCAPE '\\'`).
fn escape_like(literal: &str) -> String {
    let mut out = String::with_capacity(literal.len());
    for ch in literal.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// A compiled SQL fragment: text with `?` placeholders plus its bind values.
struct Fragment {
    sql: String,
    params: Vec<DbValue>,
}

/// Compile a predicate closure body expression to a SQL `WHERE` fragment.
struct PredicateCompiler<'a, 'c, 's> {
    ctx: &'a mut dyn StdContext,
    param: &'c str,
    /// The model's derived schema, used to validate that a property reference
    /// is well-typed for the SQL shape it lowers to. `None` when the model type
    /// is unknown at compile time (e.g. an untyped `#Predicate`), in which case
    /// validation is skipped rather than fabricating an error.
    schema: Option<&'s ModelSchema>,
}

impl<'s> PredicateCompiler<'_, '_, 's> {
    /// Resolve a referenced property to its column in the model schema. Returns
    /// `Ok(None)` when no schema is available (validation disabled); an unknown
    /// property with a schema present is a clear error.
    fn column(&self, name: &str) -> Result<Option<&'s Column>, StdError> {
        match self.schema {
            Some(schema) => match schema.columns.iter().find(|c| c.name == name) {
                Some(col) => Ok(Some(col)),
                None => Err(predicate_error(format!(
                    "'{name}' is not a stored property of {}",
                    schema.type_name
                ))),
            },
            None => Ok(None),
        }
    }

    fn compile(&mut self, node: &Node<'static>) -> Result<Fragment, StdError> {
        match node.kind() {
            // A parenthesised single expression (should the frontend ever wrap
            // one in a `TupleExpr`) is transparent.
            NodeKind::TupleExpr if node.children().count() == 1 => {
                let inner = node.children().next().unwrap();
                self.compile(&inner)
            }
            NodeKind::PrefixExpr if node.text().as_deref() == Some("!") => {
                let inner = node
                    .children()
                    .next()
                    .ok_or_else(|| predicate_error("`!` without an operand"))?;
                let frag = self.compile(&inner)?;
                Ok(Fragment {
                    sql: format!("(NOT {})", frag.sql),
                    params: frag.params,
                })
            }
            // A String property's `.isEmpty` (`obj.title.isEmpty`) lowers to a
            // zero-length test. `param_column` on the receiver identifies the
            // column; a non-String column is rejected.
            NodeKind::MemberExpr if node.text().as_deref() == Some("isEmpty") => {
                let receiver = node
                    .children()
                    .next()
                    .ok_or_else(|| predicate_error("`isEmpty` without a receiver"))?;
                let column = param_column(&receiver, self.param).ok_or_else(|| {
                    predicate_error("`isEmpty` may only be used on a stored String property")
                })?;
                if let Some(col) = self.column(&column)? {
                    if col.sql_type != SqlType::Text {
                        return Err(predicate_error(format!(
                            "'{column}' is not a String; `isEmpty` may only be used on a \
                             String property"
                        )));
                    }
                }
                let ident = quote_ident(&column);
                Ok(Fragment {
                    // An optional empty column may be NULL or ''; both are empty.
                    sql: format!("({ident} IS NULL OR {ident} = '')"),
                    params: vec![],
                })
            }
            // A bare boolean stored property (`obj.watched`) used in boolean
            // position lowers to `\"watched\" = 1`.
            NodeKind::MemberExpr => {
                let column = param_column(node, self.param).ok_or_else(|| {
                    predicate_error("only a stored property of the model object is supported here")
                })?;
                // A bare property in boolean position must be a `Bool` column.
                if let Some(col) = self.column(&column)? {
                    if !col.is_bool {
                        return Err(predicate_error(format!(
                            "'{column}' is not a Bool; only a Bool property may be used as a \
                             standalone condition (write an explicit comparison instead)"
                        )));
                    }
                }
                Ok(Fragment {
                    sql: format!("{} = 1", quote_ident(&column)),
                    params: vec![],
                })
            }
            NodeKind::BinaryExpr => self.compile_binary(node),
            NodeKind::CallExpr => self.compile_string_method(node),
            other => Err(predicate_error(format!(
                "unsupported expression `{other:?}` (supported: &&, ||, !, comparisons, \
                 and String contains/hasPrefix/hasSuffix)"
            ))),
        }
    }

    fn compile_binary(&mut self, node: &Node<'static>) -> Result<Fragment, StdError> {
        let op = node.text().unwrap_or_default();
        let mut kids = node.children();
        let lhs = kids
            .next()
            .ok_or_else(|| predicate_error("binary expression missing left operand"))?;
        let rhs = kids
            .next()
            .ok_or_else(|| predicate_error("binary expression missing right operand"))?;

        // Logical connectives combine two boolean fragments.
        if op == "&&" || op == "||" {
            let joiner = if op == "&&" { " AND " } else { " OR " };
            let mut left = self.compile(&lhs)?;
            let right = self.compile(&rhs)?;
            left.params.extend(right.params);
            return Ok(Fragment {
                sql: format!("({}{joiner}{})", left.sql, right.sql),
                params: left.params,
            });
        }

        let Some(sql_op) = sql_comparison_op(&op) else {
            return Err(predicate_error(format!("unsupported operator `{op}`")));
        };

        // Exactly one side must reference the model object (the column); the
        // other is evaluated eagerly to a bound parameter.
        let left_col = param_column(&lhs, self.param);
        let right_col = param_column(&rhs, self.param);
        let (column, value_node, effective_op) = match (left_col, right_col) {
            (Some(col), None) if !references_param(&rhs, self.param) => (col, rhs, sql_op),
            (None, Some(col)) if !references_param(&lhs, self.param) => {
                (col, lhs, flip_comparison(&op))
            }
            _ => {
                return Err(predicate_error(
                    "a comparison must be between one stored property of the model \
                     object and a literal or captured value",
                ))
            }
        };
        let effective_op = sql_comparison_op(effective_op).unwrap_or(effective_op);

        // Validate the property reference exists in the schema (when known).
        let col_info = self.column(&column)?;

        let value = self.ctx.eval_node(&value_node)?;
        let db_value = swift_to_db_value(&value)?;
        let ident = quote_ident(&column);

        // `== nil` / `!= nil` lower to `IS NULL` / `IS NOT NULL`.
        if matches!(db_value, DbValue::Null) {
            // Comparing to nil is only meaningful for an optional property.
            if let Some(col) = col_info {
                if col.not_null {
                    return Err(predicate_error(format!(
                        "'{column}' is not optional; only an optional property may be \
                         compared to nil"
                    )));
                }
            }
            return match effective_op {
                "=" => Ok(Fragment {
                    sql: format!("{ident} IS NULL"),
                    params: vec![],
                }),
                "<>" => Ok(Fragment {
                    sql: format!("{ident} IS NOT NULL"),
                    params: vec![],
                }),
                _ => Err(predicate_error("only == / != may compare against nil")),
            };
        }
        Ok(Fragment {
            sql: format!("{ident} {effective_op} ?"),
            params: vec![db_value],
        })
    }

    /// `obj.text.contains(\"x\")` / `hasPrefix` / `hasSuffix` → `LIKE`.
    fn compile_string_method(&mut self, node: &Node<'static>) -> Result<Fragment, StdError> {
        let mut kids = node.children();
        let callee = kids
            .next()
            .ok_or_else(|| predicate_error("call without a callee"))?;
        if callee.kind() != NodeKind::MemberExpr {
            return Err(predicate_error("unsupported call in predicate"));
        }
        let method = callee.text().unwrap_or_default();
        let receiver = callee
            .children()
            .next()
            .ok_or_else(|| predicate_error("method call without a receiver"))?;
        // Collection membership `collection.contains(obj.prop)` — the receiver
        // is a captured/literal array and the argument is the model column —
        // lowers to `prop IN (?, …)`. (Distinct from `obj.text.contains("x")`,
        // where the receiver is the column; handled below.)
        if method == "contains" && param_column(&receiver, self.param).is_none() {
            return self.compile_collection_contains(node, &receiver);
        }
        let column = param_column(&receiver, self.param).ok_or_else(|| {
            predicate_error("string predicate must call the method on a stored property")
        })?;
        // `contains`/`hasPrefix`/`hasSuffix` lower to `LIKE`, valid only on a
        // `String` (TEXT) column.
        if let Some(col) = self.column(&column)? {
            if col.sql_type != SqlType::Text {
                return Err(predicate_error(format!(
                    "'{column}' is not a String; `{method}` may only be used on a \
                     String property"
                )));
            }
        }
        let arg = kids
            .next()
            .ok_or_else(|| predicate_error(format!("`{method}` requires one argument")))?;
        if references_param(&arg, self.param) {
            return Err(predicate_error(
                "the argument may not reference the model object",
            ));
        }
        if kids.next().is_some() {
            return Err(predicate_error(format!(
                "`{method}` takes exactly one argument"
            )));
        }
        let value = self.ctx.eval_node(&arg)?;
        let needle = as_string(&value)
            .ok_or_else(|| predicate_error(format!("`{method}` expects a String argument")))?;
        let pattern = match method.as_str() {
            "contains" => format!("%{}%", escape_like(&needle)),
            "hasPrefix" => format!("{}%", escape_like(&needle)),
            "hasSuffix" => format!("%{}", escape_like(&needle)),
            other => {
                return Err(predicate_error(format!(
                "unsupported String method `{other}` (supported: contains, hasPrefix, hasSuffix)"
            )))
            }
        };
        Ok(Fragment {
            sql: format!("{} LIKE ? ESCAPE '\\'", quote_ident(&column)),
            params: vec![DbValue::Text(pattern)],
        })
    }

    /// `collection.contains(obj.prop)` → `prop IN (?, …)`. The receiver is a
    /// captured/literal collection (evaluated eagerly to its elements); the sole
    /// argument must be a stored property of the model object. An empty
    /// collection yields a constant-false fragment (`IN ()` is a syntax error in
    /// SQL, and nothing is a member of the empty set).
    fn compile_collection_contains(
        &mut self,
        node: &Node<'static>,
        receiver: &Node<'static>,
    ) -> Result<Fragment, StdError> {
        let mut kids = node.children();
        kids.next(); // callee (already inspected)
        let arg = kids
            .next()
            .ok_or_else(|| predicate_error("`contains` requires one argument"))?;
        if kids.next().is_some() {
            return Err(predicate_error("`contains` takes exactly one argument"));
        }
        let column = param_column(&arg, self.param).ok_or_else(|| {
            predicate_error(
                "collection `contains` must test a stored property of the model object \
                 (`list.contains(obj.prop)`)",
            )
        })?;
        // Validate the column exists (when a schema is known).
        self.column(&column)?;
        let value = self.ctx.eval_node(receiver)?;
        let elements = match &value {
            SwiftValue::Array(items) => items.as_ref().clone(),
            SwiftValue::Set(items) => items.iter().cloned().collect(),
            _ => {
                return Err(predicate_error(
                    "collection `contains` requires an array or set of literal/captured values",
                ))
            }
        };
        if elements.is_empty() {
            // Nothing is a member of the empty set.
            return Ok(Fragment {
                sql: "0".to_string(),
                params: vec![],
            });
        }
        let mut params = Vec::with_capacity(elements.len());
        for element in &elements {
            params.push(swift_to_db_value(element)?);
        }
        let placeholders = vec!["?"; params.len()].join(", ");
        Ok(Fragment {
            sql: format!("{} IN ({placeholders})", quote_ident(&column)),
            params,
        })
    }
}

/// The result expression of a predicate closure body (its single boolean
/// expression), unwrapping an `ExprStmt`/`ReturnStmt` wrapper.
fn closure_result_expr(closure: &Node<'static>) -> Option<Node<'static>> {
    let last = closure
        .children()
        .filter(|c| c.kind() != NodeKind::Param)
        .last()?;
    match last.kind() {
        NodeKind::ExprStmt | NodeKind::ReturnStmt => last.children().next(),
        _ => Some(last),
    }
}

fn predicate_macro(ctx: &mut dyn StdContext, node: &Node<'static>) -> StdResult {
    let type_name = node
        .children()
        .find(|c| c.kind() == NodeKind::TypeRef)
        .and_then(|c| c.text());
    let closure = node
        .children()
        .find(|c| c.kind() == NodeKind::ClosureExpr)
        .ok_or_else(|| predicate_error("expected a closure body, `#Predicate<T> { obj in … }`"))?;
    let param = closure
        .children()
        .find(|c| c.kind() == NodeKind::Param)
        .and_then(|c| c.text())
        .ok_or_else(|| predicate_error("the closure must name its parameter"))?;
    let body = closure_result_expr(&closure)
        .ok_or_else(|| predicate_error("the closure has no boolean expression"))?;

    // Derive the model schema so property references can be type-checked. If the
    // type is unwritten or its schema can't be derived (e.g. no such @Model),
    // validation is skipped — container creation surfaces schema errors clearly.
    let schema = match &type_name {
        Some(t) => derive_schema(&*ctx, t).ok(),
        None => None,
    };

    let fragment = {
        let mut compiler = PredicateCompiler {
            ctx,
            param: &param,
            schema: schema.as_ref(),
        };
        compiler.compile(&body)?
    };

    Ok(make_object(
        "Predicate",
        vec![
            ("__where".into(), SwiftValue::Str(fragment.sql)),
            (
                "__params".into(),
                SwiftValue::Str(encode_params(&fragment.params)),
            ),
            (
                "__type".into(),
                type_name.map(SwiftValue::Str).unwrap_or(SwiftValue::Nil),
            ),
        ],
    ))
}

// ---------------------------------------------------------------------------
// SortDescriptor(\.key, order:)  and  FetchDescriptor(predicate:sortBy:)
// ---------------------------------------------------------------------------

fn sort_descriptor_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut column: Option<String> = None;
    let mut order = "forward".to_string();
    for arg in &args {
        match arg.label.as_deref() {
            Some("order") => {
                order = match &arg.value {
                    SwiftValue::Enum(e) => e.case.clone(),
                    other => as_string(other).unwrap_or_else(|| "forward".to_string()),
                };
            }
            _ => {
                if let Some(comps) = ctx.key_path_components(&arg.value) {
                    if !comps.is_empty() {
                        column = Some(comps.join("."));
                    }
                }
            }
        }
    }
    let Some(column) = column else {
        return Err(type_error(
            "SortDescriptor requires a key path naming a stored property, e.g. SortDescriptor(\\.year)",
        ));
    };
    Ok(make_object(
        "SortDescriptor",
        vec![
            ("column".into(), SwiftValue::Str(column)),
            ("order".into(), SwiftValue::Str(order)),
        ],
    ))
}

fn fetch_descriptor_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut where_sql = String::new();
    let mut params_json = encode_params(&[]);
    let mut type_name = SwiftValue::Nil;
    let mut sort_by = SwiftValue::Array(Rc::new(Vec::new()));
    let mut fetch_limit = SwiftValue::Nil;
    let mut fetch_offset = SwiftValue::int(0);
    let mut predicate = SwiftValue::Nil;
    for arg in &args {
        match arg.label.as_deref() {
            Some("predicate") | None => {
                if let SwiftValue::Object(o) = &arg.value {
                    let o = o.borrow();
                    if o.class_name == "Predicate" {
                        where_sql = o.get("__where").and_then(as_string).unwrap_or_default();
                        if let Some(p) = o.get("__params").and_then(as_string) {
                            params_json = p;
                        }
                        if let Some(t) = o.get("__type") {
                            type_name = t.clone();
                        }
                        predicate = arg.value.clone();
                    }
                }
            }
            Some("sortBy") => {
                if matches!(&arg.value, SwiftValue::Array(_)) {
                    sort_by = arg.value.clone();
                }
            }
            Some("fetchLimit") => {
                if matches!(&arg.value, SwiftValue::Int(_)) {
                    fetch_limit = arg.value.clone();
                }
            }
            Some("fetchOffset") => {
                if matches!(&arg.value, SwiftValue::Int(_)) {
                    fetch_offset = arg.value.clone();
                }
            }
            _ => {}
        }
    }
    Ok(make_object(
        "FetchDescriptor",
        vec![
            ("__where".into(), SwiftValue::Str(where_sql)),
            ("__params".into(), SwiftValue::Str(params_json)),
            ("__type".into(), type_name),
            ("sortBy".into(), sort_by),
            ("fetchLimit".into(), fetch_limit),
            ("fetchOffset".into(), fetch_offset),
            ("predicate".into(), predicate),
        ],
    ))
}

fn fetch_descriptor_property(recv: &SwiftValue, field: &str) -> StdResult {
    let SwiftValue::Object(o) = recv else {
        return Err(type_error(
            "FetchDescriptor property on a non-descriptor value",
        ));
    };
    Ok(o.borrow().get(field).cloned().unwrap_or(SwiftValue::Nil))
}

fn config_property(recv: &SwiftValue, field: &str) -> StdResult {
    let SwiftValue::Object(o) = recv else {
        return Err(type_error(
            "ModelConfiguration property on a non-configuration value",
        ));
    };
    Ok(o.borrow().get(field).cloned().unwrap_or(SwiftValue::Nil))
}

// ---------------------------------------------------------------------------
// context.fetch(FetchDescriptor)
// ---------------------------------------------------------------------------

/// A read-only snapshot of a `FetchDescriptor` object's compiled parts.
struct FetchPlan {
    where_sql: String,
    params: Vec<DbValue>,
    type_name: Option<String>,
    order_by: Vec<(String, bool)>, // (column, reverse)
    limit: Option<i64>,
    offset: i64,
}

fn read_fetch_plan(descriptor: &SwiftValue) -> Result<FetchPlan, StdError> {
    let SwiftValue::Object(o) = descriptor else {
        return Err(type_error(
            "ModelContext.fetch(_:) expects a FetchDescriptor argument",
        ));
    };
    let o = o.borrow();
    if o.class_name != "FetchDescriptor" {
        return Err(type_error(
            "ModelContext.fetch(_:) expects a FetchDescriptor argument",
        ));
    }
    let where_sql = o.get("__where").and_then(as_string).unwrap_or_default();
    let params = o
        .get("__params")
        .and_then(as_string)
        .map(|s| db::decode_params(&s).unwrap_or_default())
        .unwrap_or_default();
    let type_name = o.get("__type").and_then(as_string);
    let mut order_by = Vec::new();
    if let Some(SwiftValue::Array(items)) = o.get("sortBy") {
        for item in items.iter() {
            if let SwiftValue::Object(sd) = item {
                let sd = sd.borrow();
                if sd.class_name == "SortDescriptor" {
                    if let Some(col) = sd.get("column").and_then(as_string) {
                        let reverse =
                            sd.get("order").and_then(as_string).as_deref() == Some("reverse");
                        order_by.push((col, reverse));
                    }
                }
            }
        }
    }
    let limit = match o.get("fetchLimit") {
        Some(SwiftValue::Int(i)) => i64::try_from(i.raw).ok(),
        _ => None,
    };
    let offset = match o.get("fetchOffset") {
        Some(SwiftValue::Int(i)) => i64::try_from(i.raw).unwrap_or(0).max(0),
        _ => 0,
    };
    Ok(FetchPlan {
        where_sql,
        params,
        type_name,
        order_by,
        limit,
        offset,
    })
}

fn select_sql(schema: &ModelSchema, plan: &FetchPlan) -> String {
    let mut cols = vec!["rowid".to_string()];
    cols.extend(schema.columns.iter().map(|c| quote_ident(&c.name)));
    let mut sql = format!(
        "SELECT {} FROM {}",
        cols.join(", "),
        quote_ident(&schema.table)
    );
    if !plan.where_sql.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&plan.where_sql);
    }
    if !plan.order_by.is_empty() {
        let terms: Vec<String> = plan
            .order_by
            .iter()
            .map(|(col, reverse)| {
                format!(
                    "{} {}",
                    quote_ident(col),
                    if *reverse { "DESC" } else { "ASC" }
                )
            })
            .collect();
        sql.push_str(" ORDER BY ");
        sql.push_str(&terms.join(", "));
    }
    // SQLite requires a LIMIT before OFFSET; use `LIMIT -1` (unbounded) when
    // only an offset is set so pagination works without an explicit limit.
    match (plan.limit, plan.offset) {
        (Some(limit), 0) => sql.push_str(&format!(" LIMIT {limit}")),
        (Some(limit), offset) => sql.push_str(&format!(" LIMIT {limit} OFFSET {offset}")),
        (None, offset) if offset > 0 => sql.push_str(&format!(" LIMIT -1 OFFSET {offset}")),
        (None, _) => {}
    }
    sql
}

/// Decode a fetched column value into the Swift value its property expects.
fn db_to_swift(value: &DbValue, col: &Column) -> SwiftValue {
    match value {
        DbValue::Null => SwiftValue::Nil,
        DbValue::Int(i) => {
            if col.is_bool {
                SwiftValue::Bool(*i != 0)
            } else {
                SwiftValue::int(*i as i128)
            }
        }
        DbValue::Real(d) => SwiftValue::Double(*d),
        DbValue::Text(s) => SwiftValue::Str(s.clone()),
        DbValue::Blob(_) => SwiftValue::Nil,
    }
}

fn context_fetch(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let id = context_id(&recv)?;
    let descriptor = args
        .first()
        .cloned()
        .ok_or_else(|| type_error("ModelContext.fetch(_:) expects a FetchDescriptor argument"))?;
    let plan = read_fetch_plan(&descriptor)?;
    let iid = ctx.interpreter_id();

    // Take the context state out so the host query call doesn't alias the
    // thread-local borrow; always put it back.
    let mut state = with_state(iid, |s| s.contexts.remove(&id))
        .ok_or_else(|| type_error("ModelContext.fetch(): unknown context"))?;
    let result = fetch_rows(ctx, &mut state, &plan);
    with_state(iid, |s| {
        s.contexts.insert(id, state);
    });
    let objects = result?;
    Ok(Outcome {
        result: SwiftValue::Array(Rc::new(objects)),
        receiver: recv,
    })
}

// ---------------------------------------------------------------------------
// fetchCount / change tracking / rollback / transaction
// ---------------------------------------------------------------------------

/// `ModelContext.fetchCount(_:) -> Int` — the number of rows a matching
/// `fetch(_:)` would return. Mirrors `fetch`'s in-context semantics exactly
/// (pending-deleted rows excluded) by running the same plan and counting.
fn context_fetch_count(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let id = context_id(&recv)?;
    let descriptor = args.first().cloned().ok_or_else(|| {
        type_error("ModelContext.fetchCount(_:) expects a FetchDescriptor argument")
    })?;
    let plan = read_fetch_plan(&descriptor)?;
    let iid = ctx.interpreter_id();
    let mut state = with_state(iid, |s| s.contexts.remove(&id))
        .ok_or_else(|| type_error("ModelContext.fetchCount(): unknown context"))?;
    let result = fetch_rows(ctx, &mut state, &plan);
    with_state(iid, |s| {
        s.contexts.insert(id, state);
    });
    let objects = result?;
    Ok(Outcome {
        result: SwiftValue::int(objects.len() as i128),
        receiver: recv,
    })
}

/// Whether a tracked (persisted) object's current field values differ from the
/// snapshot taken at its last flush — i.e. it is dirty and would UPDATE on save.
/// Encoding errors are treated as "not dirty" so a change-tracking query never
/// throws for a transiently invalid field (`save()` reports it authoritatively).
fn tracked_is_dirty(tracked: &Tracked, schemas: &[ModelSchema]) -> bool {
    let Ok(schema) = schema_for(schemas, &tracked.obj.borrow().class_name) else {
        return false;
    };
    match row_values(&tracked.obj.borrow(), schema) {
        Ok(values) => values != tracked.snapshot,
        Err(_) => false,
    }
}

fn context_has_changes(ctx: &mut dyn StdContext, recv: SwiftValue) -> StdResult {
    let id = context_id(&recv)?;
    let changed = with_state(ctx.interpreter_id(), |s| {
        let Some(state) = s.contexts.get(&id) else {
            return false;
        };
        !state.inserted.is_empty()
            || !state.deleted.is_empty()
            || state
                .tracked
                .iter()
                .any(|t| tracked_is_dirty(t, &state.schemas))
    });
    Ok(SwiftValue::Bool(changed))
}

fn context_inserted_models(ctx: &mut dyn StdContext, recv: SwiftValue) -> StdResult {
    let id = context_id(&recv)?;
    let objects = with_state(ctx.interpreter_id(), |s| {
        s.contexts.get(&id).map_or_else(Vec::new, |state| {
            state
                .inserted
                .iter()
                .map(|o| SwiftValue::Object(Rc::clone(o)))
                .collect()
        })
    });
    Ok(SwiftValue::Array(Rc::new(objects)))
}

fn context_changed_models(ctx: &mut dyn StdContext, recv: SwiftValue) -> StdResult {
    let id = context_id(&recv)?;
    let objects = with_state(ctx.interpreter_id(), |s| {
        s.contexts.get(&id).map_or_else(Vec::new, |state| {
            state
                .tracked
                .iter()
                .filter(|t| tracked_is_dirty(t, &state.schemas))
                .map(|t| SwiftValue::Object(Rc::clone(&t.obj)))
                .collect()
        })
    });
    Ok(SwiftValue::Array(Rc::new(objects)))
}

fn context_deleted_models(ctx: &mut dyn StdContext, recv: SwiftValue) -> StdResult {
    let id = context_id(&recv)?;
    let objects = with_state(ctx.interpreter_id(), |s| {
        s.contexts.get(&id).map_or_else(Vec::new, |state| {
            state
                .deleted
                .iter()
                .map(|(o, _)| SwiftValue::Object(Rc::clone(o)))
                .collect()
        })
    });
    Ok(SwiftValue::Array(Rc::new(objects)))
}

/// Restore a tracked object's fields to its last-flushed snapshot, undoing any
/// in-memory edits so a rolled-back dirty object matches the store again.
fn restore_snapshot(tracked: &Tracked, schemas: &[ModelSchema]) {
    let Ok(schema) = schema_for(schemas, &tracked.obj.borrow().class_name) else {
        return;
    };
    let mut obj = tracked.obj.borrow_mut();
    for (col, value) in schema.columns.iter().zip(&tracked.snapshot) {
        obj.set(&col.name, db_to_swift(value, col));
    }
}

/// Discard every un-saved change: drop pending inserts, un-mark pending
/// deletes (returning them to the tracked set), and revert dirty tracked
/// objects to their last-flushed snapshot. Nothing is written to the store.
fn rollback_state(state: &mut ContextState) {
    state.inserted.clear();
    let schemas = Rc::clone(&state.schemas);
    for (obj, rowid) in state.deleted.drain(..).collect::<Vec<_>>() {
        let snapshot = schema_for(&schemas, &obj.borrow().class_name)
            .and_then(|schema| row_values(&obj.borrow(), schema))
            .unwrap_or_default();
        state
            .by_identity
            .insert((obj.borrow().class_name.clone(), rowid), Rc::clone(&obj));
        state.tracked.push(Tracked {
            obj,
            rowid,
            snapshot,
        });
    }
    for tracked in &state.tracked {
        restore_snapshot(tracked, &schemas);
    }
}

fn context_rollback(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    _args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let id = context_id(&recv)?;
    with_state(ctx.interpreter_id(), |s| {
        if let Some(state) = s.contexts.get_mut(&id) {
            rollback_state(state);
        }
    });
    Ok(Outcome {
        result: SwiftValue::Void,
        receiver: recv,
    })
}

/// `ModelContext.transaction(_ block:)` — run `block`, then `save()` the
/// context so the whole body commits atomically. If `block` throws, discard
/// its partial changes (`rollback`) and re-propagate, so a failed transaction
/// leaves the context clean.
fn context_transaction(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<SwiftValue>,
) -> Result<Outcome, StdError> {
    let id = context_id(&recv)?;
    let closure = args.iter().find_map(|a| match a {
        SwiftValue::Closure(cid) => Some(*cid),
        _ => None,
    });
    let Some(closure) = closure else {
        return Err(type_error(
            "ModelContext.transaction(_:) expects a closure argument",
        ));
    };
    match ctx.call_closure(closure, vec![]) {
        Ok(_) => {
            // Commit the body's accumulated changes.
            let outcome = context_save(ctx, recv, vec![])?;
            Ok(outcome)
        }
        Err(err) => {
            // Body failed: discard its partial changes and re-propagate.
            with_state(ctx.interpreter_id(), |s| {
                if let Some(state) = s.contexts.get_mut(&id) {
                    rollback_state(state);
                }
            });
            Err(err)
        }
    }
}

fn fetch_rows(
    ctx: &mut dyn StdContext,
    state: &mut ContextState,
    plan: &FetchPlan,
) -> Result<Vec<SwiftValue>, StdError> {
    // Resolve the model type: the predicate's `<T>`, else the sole registered
    // schema, else a clear diagnostic (never a silent wrong table).
    let type_name = match &plan.type_name {
        Some(t) => t.clone(),
        None if state.schemas.len() == 1 => state.schemas[0].type_name.clone(),
        None => {
            return Err(type_error(
                "SwiftData: cannot infer the model type to fetch; use FetchDescriptor with a \
                 #Predicate<T> (this container registers several model types)",
            ))
        }
    };
    let schema = schema_for(&state.schemas, &type_name)?;
    let sql = select_sql(schema, plan);
    let rows = query(ctx, state.handle, &sql, &plan.params)?;

    // Rows marked for deletion in this context but not yet committed are still
    // physically present in the store, so the SELECT returns them. SwiftData
    // excludes pending-deleted objects from fetch results within the context,
    // so filter them out by rowid (scoped to this fetch's model type).
    let deleted_rowids: std::collections::HashSet<i64> = state
        .deleted
        .iter()
        .filter(|(obj, _)| obj.borrow().class_name == type_name)
        .map(|(_, rowid)| *rowid)
        .collect();

    let mut objects = Vec::with_capacity(rows.len());
    for row in &rows {
        let rowid = row
            .iter()
            .find(|(name, _)| name == "rowid")
            .and_then(|(_, v)| match v {
                DbValue::Int(i) => Some(*i),
                _ => None,
            })
            .ok_or_else(|| type_error("SwiftData: fetched row is missing its rowid"))?;

        // Exclude objects pending deletion in this context.
        if deleted_rowids.contains(&rowid) {
            continue;
        }

        // Identity map: fetching the same row twice returns the same instance.
        // Keyed by `(type name, rowid)` so a row in another table with the same
        // rowid can't alias this one.
        if let Some(existing) = state.by_identity.get(&(type_name.clone(), rowid)) {
            objects.push(SwiftValue::Object(Rc::clone(existing)));
            continue;
        }

        let schema = schema_for(&state.schemas, &type_name)?;
        let mut fields = Vec::with_capacity(schema.columns.len());
        let mut snapshot = Vec::with_capacity(schema.columns.len());
        for col in &schema.columns {
            let db_value = row
                .iter()
                .find(|(name, _)| name == &col.name)
                .map(|(_, v)| v.clone())
                .unwrap_or(DbValue::Null);
            fields.push((col.name.clone(), db_to_swift(&db_value, col)));
            snapshot.push(db_value);
        }
        let obj = Rc::new(RefCell::new(ClassObj {
            class_name: type_name.clone(),
            fields,
        }));
        state
            .by_identity
            .insert((type_name.clone(), rowid), Rc::clone(&obj));
        state.tracked.push(Tracked {
            obj: Rc::clone(&obj),
            rowid,
            snapshot,
        });
        objects.push(SwiftValue::Object(obj));
    }
    Ok(objects)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Write};
    use tswift_core::json::{self, Json};
    use tswift_core::{NominalProperty, NominalTypeInfo};

    /// A recorded host call: the op name plus, for execute, the SQL and decoded
    /// bind parameters.
    #[derive(Debug, Clone, PartialEq)]
    enum Call {
        Open(String),
        Close(i64),
        Begin,
        Commit,
        Rollback,
        Execute(String, Vec<DbValue>),
        Query(String, Vec<DbValue>),
    }

    /// A mock [`StdContext`] that records the `tswift.db.*` wire traffic the
    /// SwiftData surface emits, and answers `nominal_type_info` from a fixed
    /// schema — so tests assert the *exact* SQL + param sequence without a
    /// frontend or a real database (house style: direct calls against a small
    /// mock, per `user_defaults.rs`).
    struct MockCtx {
        /// A distinct per-mock identity, mirroring a real interpreter's, so the
        /// registry scopes each mock to its own bucket.
        id: u64,
        available: bool,
        info: Vec<(String, NominalTypeInfo)>,
        calls: Vec<Call>,
        next_rowid: i64,
        /// SQL substring that, when executed, makes the host reply `$thrown`.
        fail_on: Option<String>,
        /// When true, `tswift.db.commit` replies `$thrown` (a commit-time
        /// failure), exercising the flush's commit-error rollback path.
        fail_commit: bool,
        /// Canned `tswift.db.query` reply rows, keyed by a SQL substring match.
        query_rows: Vec<(String, Vec<DbRow>)>,
        /// Values a captured identifier resolves to inside `eval_node`.
        captures: HashMap<String, SwiftValue>,
        sink: io::Sink,
    }

    impl MockCtx {
        fn new(available: bool) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(1);
            MockCtx {
                id: NEXT.fetch_add(1, Ordering::Relaxed),
                available,
                info: Vec::new(),
                calls: Vec::new(),
                next_rowid: 0,
                fail_on: None,
                fail_commit: false,
                query_rows: Vec::new(),
                captures: HashMap::new(),
                sink: io::sink(),
            }
        }

        fn with_model(mut self, name: &str, props: &[(&str, &str)]) -> Self {
            self.info.push((
                name.to_string(),
                NominalTypeInfo {
                    attributes: vec!["Model".to_string()],
                    stored: props
                        .iter()
                        .map(|(n, t)| NominalProperty {
                            name: n.to_string(),
                            declared_type: Some(t.to_string()),
                        })
                        .collect(),
                },
            ));
            self
        }

        fn executes(&self) -> Vec<(String, Vec<DbValue>)> {
            self.calls
                .iter()
                .filter_map(|c| match c {
                    Call::Execute(sql, p) => Some((sql.clone(), p.clone())),
                    _ => None,
                })
                .collect()
        }
    }

    impl StdContext for MockCtx {
        fn interpreter_id(&self) -> u64 {
            self.id
        }
        fn call_closure(&mut self, _id: usize, _args: Vec<SwiftValue>) -> StdResult {
            unreachable!("SwiftData surface calls no closures")
        }
        fn out(&mut self) -> &mut dyn Write {
            &mut self.sink
        }
        fn is_host_fn(&self, name: &str) -> bool {
            self.available && name.starts_with("tswift.db.")
        }
        fn nominal_type_info(&self, type_name: &str) -> Option<NominalTypeInfo> {
            self.info
                .iter()
                .find(|(n, _)| n == type_name)
                .map(|(_, i)| i.clone())
        }
        fn call_host_fn(
            &mut self,
            name: &str,
            args: Vec<(Option<String>, SwiftValue)>,
        ) -> StdResult {
            let arg_str = |i: usize| match args.get(i).map(|(_, v)| v) {
                Some(SwiftValue::Str(s)) => s.clone(),
                _ => String::new(),
            };
            match name {
                db::OP_OPEN => {
                    self.calls.push(Call::Open(arg_str(0)));
                    Ok(SwiftValue::int(1))
                }
                db::OP_CLOSE => {
                    let handle = match args.first().map(|(_, v)| v) {
                        Some(SwiftValue::Int(i)) => i64::try_from(i.raw).unwrap_or(0),
                        _ => 0,
                    };
                    self.calls.push(Call::Close(handle));
                    Ok(SwiftValue::Void)
                }
                db::OP_BEGIN => {
                    self.calls.push(Call::Begin);
                    Ok(SwiftValue::Void)
                }
                db::OP_COMMIT => {
                    self.calls.push(Call::Commit);
                    if self.fail_commit {
                        return Err(self.throw(host_error("commit failed")));
                    }
                    Ok(SwiftValue::Void)
                }
                db::OP_ROLLBACK => {
                    self.calls.push(Call::Rollback);
                    Ok(SwiftValue::Void)
                }
                db::OP_EXECUTE => {
                    let sql = arg_str(1);
                    let params = db::decode_params(&arg_str(2)).unwrap();
                    self.calls.push(Call::Execute(sql.clone(), params));
                    if let Some(needle) = &self.fail_on {
                        if sql.contains(needle.as_str()) {
                            return Err(self.throw(host_error("boom")));
                        }
                    }
                    // Only INSERTs advance last_insert_rowid, like SQLite.
                    if sql.starts_with("INSERT") {
                        self.next_rowid += 1;
                    }
                    Ok(SwiftValue::Str(
                        ExecResult {
                            rows_affected: 1,
                            last_insert_rowid: self.next_rowid,
                        }
                        .encode(),
                    ))
                }
                db::OP_QUERY => {
                    let sql = arg_str(1);
                    let params = db::decode_params(&arg_str(2)).unwrap();
                    self.calls.push(Call::Query(sql.clone(), params));
                    if let Some(needle) = &self.fail_on {
                        if sql.contains(needle.as_str()) {
                            return Err(self.throw(host_error("boom")));
                        }
                    }
                    let rows = self
                        .query_rows
                        .iter()
                        .find(|(needle, _)| sql.contains(needle.as_str()))
                        .map(|(_, rows)| rows.clone())
                        .unwrap_or_default();
                    Ok(SwiftValue::Str(db::encode_rows(&rows)))
                }
                other => panic!("unexpected host fn {other}"),
            }
        }

        fn key_path_components(&self, value: &SwiftValue) -> Option<Vec<String>> {
            // Test convention: a key path is spelled `Str("kp:col")`.
            match value {
                SwiftValue::Str(s) => s.strip_prefix("kp:").map(|c| vec![c.to_string()]),
                _ => None,
            }
        }

        fn eval_node(&mut self, node: &Node<'static>) -> StdResult {
            // A minimal literal/captured-identifier evaluator — enough for the
            // non-column side of a predicate comparison in these tests.
            match node.kind() {
                NodeKind::IntegerLiteral => Ok(SwiftValue::int(node.int().unwrap_or(0) as i128)),
                NodeKind::FloatLiteral => Ok(SwiftValue::Double(node.float().unwrap_or(0.0))),
                NodeKind::BoolLiteral => Ok(SwiftValue::Bool(node.bool().unwrap_or(false))),
                NodeKind::NilLiteral => Ok(SwiftValue::Nil),
                NodeKind::StringLiteral => {
                    let raw = node.text().unwrap_or_default();
                    let inner = raw
                        .strip_prefix('"')
                        .and_then(|s| s.strip_suffix('"'))
                        .unwrap_or(&raw)
                        .to_string();
                    Ok(SwiftValue::Str(inner))
                }
                NodeKind::IdentExpr => {
                    let name = node.text().unwrap_or_default();
                    self.captures
                        .get(&name)
                        .cloned()
                        .ok_or_else(|| type_error(format!("unknown capture `{name}`")))
                }
                other => Err(type_error(format!("mock eval_node: unsupported {other:?}"))),
            }
        }
    }

    fn metatype(name: &str) -> Arg {
        Arg::positional(SwiftValue::Metatype(name.to_string()))
    }

    fn labeled(label: &str, value: SwiftValue) -> Arg {
        Arg {
            label: Some(label.to_string()),
            value,
            static_ty: None,
        }
    }

    /// Build an in-memory container for a single `Movie(title:String, year:Int)`
    /// model and return `(container_value, main_context_value)`.
    fn movie_container(ctx: &mut MockCtx) -> (SwiftValue, SwiftValue) {
        let config = model_configuration_init(
            ctx,
            vec![labeled("isStoredInMemoryOnly", SwiftValue::Bool(true))],
        )
        .unwrap();
        let container = model_container_init(
            ctx,
            vec![
                labeled("for", SwiftValue::Metatype("Movie".into())),
                labeled("configurations", config),
            ],
        )
        .unwrap();
        let main = container_main_context(ctx, container.clone()).unwrap();
        (container, main)
    }

    fn movie(title: &str, year: i128) -> SwiftValue {
        make_object(
            "Movie",
            vec![
                ("title".into(), SwiftValue::Str(title.into())),
                ("year".into(), SwiftValue::int(year)),
            ],
        )
    }

    #[test]
    fn container_open_creates_table_in_memory() {
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        movie_container(&mut ctx);
        assert_eq!(ctx.calls[0], Call::Open(":memory:".into()));
        assert_eq!(
            ctx.calls[1],
            Call::Execute(
                "CREATE TABLE IF NOT EXISTS \"Movie\" (\"title\" TEXT NOT NULL, \"year\" INTEGER NOT NULL)"
                    .into(),
                vec![]
            )
        );
    }

    #[test]
    fn insert_save_emits_transactional_insert() {
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        let (_c, main) = movie_container(&mut ctx);
        let m = movie("Arrival", 2016);
        context_insert(&mut ctx, main.clone(), vec![m.clone()]).unwrap();
        let before = ctx.calls.len();
        context_save(&mut ctx, main, vec![]).unwrap();
        assert_eq!(ctx.calls[before], Call::Begin);
        assert_eq!(
            ctx.calls[before + 1],
            Call::Execute(
                "INSERT INTO \"Movie\" (\"title\", \"year\") VALUES (?, ?)".into(),
                vec![DbValue::Text("Arrival".into()), DbValue::Int(2016)]
            )
        );
        assert_eq!(ctx.calls[before + 2], Call::Commit);
    }

    #[test]
    fn mutate_then_save_emits_update_by_rowid() {
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        let (_c, main) = movie_container(&mut ctx);
        let m = movie("Arrival", 2016);
        context_insert(&mut ctx, main.clone(), vec![m.clone()]).unwrap();
        context_save(&mut ctx, main.clone(), vec![]).unwrap();
        // Mutate the live object, then save again.
        if let SwiftValue::Object(o) = &m {
            o.borrow_mut().set("year", SwiftValue::int(2017));
        }
        let before = ctx.calls.len();
        context_save(&mut ctx, main, vec![]).unwrap();
        assert_eq!(ctx.calls[before], Call::Begin);
        assert_eq!(
            ctx.calls[before + 1],
            Call::Execute(
                "UPDATE \"Movie\" SET \"title\" = ?, \"year\" = ? WHERE rowid = ?".into(),
                vec![
                    DbValue::Text("Arrival".into()),
                    DbValue::Int(2017),
                    DbValue::Int(1)
                ]
            )
        );
        assert_eq!(ctx.calls[before + 2], Call::Commit);
    }

    #[test]
    fn unchanged_save_is_a_noop() {
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        let (_c, main) = movie_container(&mut ctx);
        let m = movie("Arrival", 2016);
        context_insert(&mut ctx, main.clone(), vec![m]).unwrap();
        context_save(&mut ctx, main.clone(), vec![]).unwrap();
        let before = ctx.calls.len();
        // No mutation: a second save issues no statements at all.
        context_save(&mut ctx, main, vec![]).unwrap();
        assert_eq!(ctx.calls.len(), before);
    }

    #[test]
    fn delete_save_emits_delete_by_rowid() {
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        let (_c, main) = movie_container(&mut ctx);
        let m = movie("Arrival", 2016);
        context_insert(&mut ctx, main.clone(), vec![m.clone()]).unwrap();
        context_save(&mut ctx, main.clone(), vec![]).unwrap();
        context_delete(&mut ctx, main.clone(), vec![m]).unwrap();
        let before = ctx.calls.len();
        context_save(&mut ctx, main, vec![]).unwrap();
        assert_eq!(ctx.calls[before], Call::Begin);
        assert_eq!(
            ctx.calls[before + 1],
            Call::Execute(
                "DELETE FROM \"Movie\" WHERE rowid = ?".into(),
                vec![DbValue::Int(1)]
            )
        );
        assert_eq!(ctx.calls[before + 2], Call::Commit);
    }

    #[test]
    fn insert_is_idempotent() {
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        let (_c, main) = movie_container(&mut ctx);
        let m = movie("Arrival", 2016);
        context_insert(&mut ctx, main.clone(), vec![m.clone()]).unwrap();
        context_insert(&mut ctx, main.clone(), vec![m]).unwrap();
        let before = ctx.calls.len();
        context_save(&mut ctx, main, vec![]).unwrap();
        // Exactly one INSERT despite two inserts of the same object.
        let inserts = ctx
            .executes()
            .into_iter()
            .filter(|(sql, _)| sql.starts_with("INSERT"))
            .count();
        assert_eq!(inserts, 1);
        assert!(ctx.calls.len() > before);
    }

    #[test]
    fn save_rolls_back_on_error() {
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        let (_c, main) = movie_container(&mut ctx);
        let m = movie("Arrival", 2016);
        context_insert(&mut ctx, main.clone(), vec![m]).unwrap();
        ctx.fail_on = Some("INSERT".into());
        let before = ctx.calls.len();
        let err = context_save(&mut ctx, main, vec![]).unwrap_err();
        assert!(matches!(err, StdError::Throw(_)));
        assert_eq!(ctx.calls[before], Call::Begin);
        // The failing INSERT, then a ROLLBACK, and no COMMIT.
        assert!(matches!(ctx.calls[before + 1], Call::Execute(_, _)));
        assert_eq!(ctx.calls[before + 2], Call::Rollback);
        assert!(!ctx.calls.contains(&Call::Commit));
    }

    #[test]
    fn container_throws_when_database_unavailable() {
        let mut ctx = MockCtx::new(false).with_model("Movie", &[("title", "String")]);
        let err = model_container_init(
            &mut ctx,
            vec![labeled("for", SwiftValue::Metatype("Movie".into()))],
        )
        .unwrap_err();
        assert!(matches!(err, StdError::Throw(_)));
        assert!(
            ctx.calls.is_empty(),
            "must not touch the db when unavailable"
        );
    }

    #[test]
    fn non_model_class_is_rejected() {
        let mut ctx = MockCtx::new(true);
        ctx.info.push((
            "Plain".into(),
            NominalTypeInfo {
                attributes: vec![],
                stored: vec![NominalProperty {
                    name: "x".into(),
                    declared_type: Some("Int".into()),
                }],
            },
        ));
        let err = model_container_init(
            &mut ctx,
            vec![labeled("for", SwiftValue::Metatype("Plain".into()))],
        )
        .unwrap_err();
        assert!(matches!(err, StdError::Throw(_)));
    }

    #[test]
    fn multiple_model_types_each_get_a_table() {
        let mut ctx = MockCtx::new(true)
            .with_model("Movie", &[("title", "String")])
            .with_model("Actor", &[("name", "String")]);
        model_container_init(
            &mut ctx,
            vec![
                labeled("for", SwiftValue::Metatype("Movie".into())),
                metatype("Actor"),
            ],
        )
        .unwrap();
        let creates: Vec<String> = ctx
            .executes()
            .into_iter()
            .map(|(sql, _)| sql)
            .filter(|s| s.starts_with("CREATE TABLE"))
            .collect();
        assert_eq!(creates.len(), 2);
        assert!(creates[0].contains("\"Movie\""));
        assert!(creates[1].contains("\"Actor\""));
    }

    #[test]
    fn optional_column_binds_null() {
        let mut ctx = MockCtx::new(true).with_model("Note", &[("body", "String?")]);
        let config = model_configuration_init(
            &mut ctx,
            vec![labeled("isStoredInMemoryOnly", SwiftValue::Bool(true))],
        )
        .unwrap();
        let container = model_container_init(
            &mut ctx,
            vec![
                labeled("for", SwiftValue::Metatype("Note".into())),
                labeled("configurations", config),
            ],
        )
        .unwrap();
        let main = container_main_context(&mut ctx, container).unwrap();
        let note = make_object("Note", vec![("body".into(), SwiftValue::Nil)]);
        context_insert(&mut ctx, main.clone(), vec![note]).unwrap();
        let before = ctx.calls.len();
        context_save(&mut ctx, main, vec![]).unwrap();
        assert_eq!(
            ctx.calls[before + 1],
            Call::Execute(
                "INSERT INTO \"Note\" (\"body\") VALUES (?)".into(),
                vec![DbValue::Null]
            )
        );
        // The optional column must be created without NOT NULL.
        let create = ctx
            .executes()
            .into_iter()
            .find(|(s, _)| s.starts_with("CREATE"))
            .unwrap()
            .0;
        assert_eq!(
            create,
            "CREATE TABLE IF NOT EXISTS \"Note\" (\"body\" TEXT)"
        );
        let _ = json::parse("null").map(|_: Json| ());
    }

    #[test]
    fn split_optional_strips_trailing_question() {
        assert_eq!(split_optional("Int"), ("Int", false));
        assert_eq!(split_optional("Int?"), ("Int", true));
        assert_eq!(split_optional("String ?"), ("String", true));
    }

    #[test]
    fn create_table_sql_maps_types_and_nullability() {
        let schema = ModelSchema {
            type_name: "Movie".into(),
            table: "Movie".into(),
            columns: vec![
                Column {
                    name: "title".into(),
                    sql_type: SqlType::Text,
                    not_null: true,
                    is_bool: false,
                },
                Column {
                    name: "rating".into(),
                    sql_type: SqlType::Real,
                    not_null: false,
                    is_bool: false,
                },
                Column {
                    name: "seen".into(),
                    sql_type: SqlType::Integer,
                    not_null: true,
                    is_bool: false,
                },
            ],
        };
        assert_eq!(
            create_table_sql(&schema),
            "CREATE TABLE IF NOT EXISTS \"Movie\" (\"title\" TEXT NOT NULL, \"rating\" REAL, \"seen\" INTEGER NOT NULL)"
        );
        assert_eq!(
            insert_sql(&schema),
            "INSERT INTO \"Movie\" (\"title\", \"rating\", \"seen\") VALUES (?, ?, ?)"
        );
        assert_eq!(
            update_sql(&schema),
            "UPDATE \"Movie\" SET \"title\" = ?, \"rating\" = ?, \"seen\" = ? WHERE rowid = ?"
        );
        assert_eq!(delete_sql(&schema), "DELETE FROM \"Movie\" WHERE rowid = ?");
    }

    #[test]
    fn encode_field_maps_values_and_optionals() {
        let text_col = Column {
            name: "t".into(),
            sql_type: SqlType::Text,
            not_null: false,
            is_bool: false,
        };
        assert_eq!(
            encode_field(&SwiftValue::Str("hi".into()), &text_col, "M").unwrap(),
            DbValue::Text("hi".into())
        );
        assert_eq!(
            encode_field(&SwiftValue::Nil, &text_col, "M").unwrap(),
            DbValue::Null
        );
        let int_col = Column {
            name: "n".into(),
            sql_type: SqlType::Integer,
            not_null: true,
            is_bool: false,
        };
        assert_eq!(
            encode_field(&SwiftValue::int(5), &int_col, "M").unwrap(),
            DbValue::Int(5)
        );
        assert_eq!(
            encode_field(&SwiftValue::Bool(true), &int_col, "M").unwrap(),
            DbValue::Int(1)
        );
        // non-optional nil is an error
        assert!(encode_field(&SwiftValue::Nil, &int_col, "M").is_err());
        // type mismatch is an error
        assert!(encode_field(&SwiftValue::Str("x".into()), &int_col, "M").is_err());
    }

    #[test]
    fn save_rolls_back_when_commit_fails() {
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        let (_c, main) = movie_container(&mut ctx);
        let m = movie("Arrival", 2016);
        context_insert(&mut ctx, main.clone(), vec![m.clone()]).unwrap();
        ctx.fail_commit = true;
        let before = ctx.calls.len();
        // The COMMIT itself fails: the error must surface (not be swallowed by
        // `?` leaving the transaction open) and a ROLLBACK must be issued.
        let err = context_save(&mut ctx, main.clone(), vec![]).unwrap_err();
        assert!(matches!(err, StdError::Throw(_)));
        assert_eq!(ctx.calls[before], Call::Begin);
        assert!(matches!(ctx.calls[before + 1], Call::Execute(_, _)));
        assert_eq!(ctx.calls[before + 2], Call::Commit);
        // A failed commit is followed by a best-effort rollback to release the
        // still-open transaction.
        assert_eq!(ctx.calls[before + 3], Call::Rollback);

        // The changes stay pending (SwiftData `save()`-failure semantics): a
        // subsequent successful save re-issues the same INSERT and commits.
        ctx.fail_commit = false;
        let before = ctx.calls.len();
        context_save(&mut ctx, main, vec![]).unwrap();
        assert_eq!(ctx.calls[before], Call::Begin);
        assert_eq!(
            ctx.calls[before + 1],
            Call::Execute(
                "INSERT INTO \"Movie\" (\"title\", \"year\") VALUES (?, ?)".into(),
                vec![DbValue::Text("Arrival".into()), DbValue::Int(2016)]
            )
        );
        assert_eq!(ctx.calls[before + 2], Call::Commit);
    }

    #[test]
    fn teardown_closes_open_handles_and_clears_registry() {
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        let iid = ctx.id;
        let (_c, main) = movie_container(&mut ctx);
        let ctxid = object_int_field(&main, "__ctxid").unwrap();
        // The context is registered before teardown.
        assert!(with_state(iid, |s| s.contexts.contains_key(&ctxid)));

        teardown_registry(&mut ctx);

        // A `tswift.db.close` was sent for the open handle (1, per the mock),
        // and the context's registry entry (indeed the whole bucket) is gone.
        assert!(ctx.calls.contains(&Call::Close(1)));
        assert!(!REGISTRY.with(|r| r.borrow().contains_key(&iid)));
    }

    #[test]
    fn teardown_of_one_interpreter_leaves_anothers_registry_intact() {
        // Two interpreters (mocks) sharing this thread, each with its own
        // container. Tearing down A must not disturb B's registry bucket or
        // its still-open handle — the concurrent-live-session invariant.
        let mut a = MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        let mut b = MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        assert_ne!(a.id, b.id, "distinct interpreters must get distinct ids");
        let (_ca, _ma) = movie_container(&mut a);
        let (_cb, main_b) = movie_container(&mut b);
        let b_ctxid = object_int_field(&main_b, "__ctxid").unwrap();

        // Tear A down.
        teardown_registry(&mut a);

        // A's bucket is gone; B's survives untouched.
        assert!(!REGISTRY.with(|r| r.borrow().contains_key(&a.id)));
        assert!(with_state(b.id, |s| s.contexts.contains_key(&b_ctxid)));

        // B's container still works after A's teardown: insert + save flushes.
        let movie = make_object(
            "Movie",
            vec![
                ("title".into(), SwiftValue::Str("Arrival".into())),
                ("year".into(), SwiftValue::int(2016)),
            ],
        );
        context_insert(&mut b, main_b.clone(), vec![movie]).unwrap();
        let before = b.calls.len();
        context_save(&mut b, main_b, vec![]).unwrap();
        assert_eq!(b.calls[before], Call::Begin);
        assert!(matches!(b.calls[before + 1], Call::Execute(_, _)));
        assert_eq!(b.calls[before + 2], Call::Commit);

        // Cleanup B's bucket so the thread-local doesn't leak into other tests
        // that may reuse this thread.
        teardown_registry(&mut b);
    }

    // ---------------------------------------------------------------------
    // Fetch path: #Predicate → SQL, SortDescriptor, FetchDescriptor, fetch.
    // ---------------------------------------------------------------------

    /// Parse `body` inside a `#Predicate<Movie> { m in … }` and return the
    /// `CompilerDirective` node (leaked to `'static`, matching how the
    /// interpreter holds AST nodes).
    fn predicate_node(body: &str) -> Node<'static> {
        typed_predicate_node("Movie", "m", body)
    }

    fn typed_predicate_node(type_name: &str, param: &str, body: &str) -> Node<'static> {
        let src = format!("let _p = #Predicate<{type_name}> {{ {param} in\n{body}\n}}\n");
        let analysis = tswift_frontend::Analysis::analyze(&src, "pred.swift").unwrap();
        let analysis: &'static tswift_frontend::Analysis = Box::leak(Box::new(analysis));
        find_kind(analysis.root(), NodeKind::CompilerDirective)
            .expect("no #Predicate directive parsed")
    }

    fn find_kind(node: Node<'static>, kind: NodeKind) -> Option<Node<'static>> {
        if node.kind() == kind {
            return Some(node);
        }
        for child in node.children() {
            if let Some(found) = find_kind(child, kind) {
                return Some(found);
            }
        }
        None
    }

    /// Compile a predicate body and return `(where_sql, params)`.
    fn compile_predicate(ctx: &mut MockCtx, body: &str) -> (String, Vec<DbValue>) {
        let node = predicate_node(body);
        let value = predicate_macro(ctx, &node).unwrap();
        let SwiftValue::Object(o) = value else {
            panic!("predicate is not an object");
        };
        let o = o.borrow();
        let where_sql = o.get("__where").and_then(as_string).unwrap();
        let params =
            db::decode_params(&o.get("__params").and_then(|v| as_string(&v)).unwrap()).unwrap();
        (where_sql, params)
    }

    #[test]
    fn predicate_compiles_comparisons_to_bound_where() {
        let mut ctx = MockCtx::new(true);
        let (sql, params) = compile_predicate(&mut ctx, "m.year > 2000");
        assert_eq!(sql, "\"year\" > ?");
        assert_eq!(params, vec![DbValue::Int(2000)]);

        // Swapped operands flip the operator.
        let (sql, params) = compile_predicate(&mut ctx, "2000 < m.year");
        assert_eq!(sql, "\"year\" > ?");
        assert_eq!(params, vec![DbValue::Int(2000)]);

        let (sql, params) = compile_predicate(&mut ctx, "m.title == \"Arrival\"");
        assert_eq!(sql, "\"title\" = ?");
        assert_eq!(params, vec![DbValue::Text("Arrival".into())]);

        let (sql, params) = compile_predicate(&mut ctx, "m.title != \"Dune\"");
        assert_eq!(sql, "\"title\" <> ?");
        assert_eq!(params, vec![DbValue::Text("Dune".into())]);
    }

    #[test]
    fn predicate_compiles_boolean_connectives_and_negation() {
        let mut ctx = MockCtx::new(true);
        let (sql, params) = compile_predicate(&mut ctx, "m.year >= 2000 && m.title == \"A\"");
        assert_eq!(sql, "(\"year\" >= ? AND \"title\" = ?)");
        assert_eq!(params, vec![DbValue::Int(2000), DbValue::Text("A".into())]);

        let (sql, _) = compile_predicate(&mut ctx, "m.year > 2000 || m.year < 1990");
        assert_eq!(sql, "(\"year\" > ? OR \"year\" < ?)");

        let (sql, _) = compile_predicate(&mut ctx, "!(m.year > 2000)");
        assert_eq!(sql, "(NOT \"year\" > ?)");

        // A bare boolean stored property lowers to `= 1`; `!` negates it.
        let (sql, params) = compile_predicate(&mut ctx, "!m.watched");
        assert_eq!(sql, "(NOT \"watched\" = 1)");
        assert!(params.is_empty());
    }

    #[test]
    fn predicate_compiles_string_methods_to_like() {
        let mut ctx = MockCtx::new(true);
        let (sql, params) = compile_predicate(&mut ctx, "m.title.contains(\"ar\")");
        assert_eq!(sql, "\"title\" LIKE ? ESCAPE '\\'");
        assert_eq!(params, vec![DbValue::Text("%ar%".into())]);

        let (sql, params) = compile_predicate(&mut ctx, "m.title.hasPrefix(\"Ar\")");
        assert_eq!(params, vec![DbValue::Text("Ar%".into())]);
        assert!(sql.contains("LIKE ?"));

        let (sql, params) = compile_predicate(&mut ctx, "m.title.hasSuffix(\"al\")");
        assert_eq!(params, vec![DbValue::Text("%al".into())]);
        assert!(sql.contains("LIKE ?"));

        // `%`/`_` in the needle are escaped so they match literally.
        let (_, params) = compile_predicate(&mut ctx, "m.title.contains(\"5%_x\")");
        assert_eq!(params, vec![DbValue::Text("%5\\%\\_x%".into())]);
    }

    #[test]
    fn string_method_rejects_extra_arguments() {
        let mut ctx = MockCtx::new(true);
        let node = predicate_node("m.title.contains(\"x\", \"y\")");
        let err = predicate_macro(&mut ctx, &node).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("exactly one argument"), "{msg}");
    }

    #[test]
    fn predicate_compiles_nil_comparisons_to_is_null() {
        let mut ctx = MockCtx::new(true);
        let (sql, params) = compile_predicate(&mut ctx, "m.body == nil");
        assert_eq!(sql, "\"body\" IS NULL");
        assert!(params.is_empty());
        let (sql, _) = compile_predicate(&mut ctx, "m.body != nil");
        assert_eq!(sql, "\"body\" IS NOT NULL");
    }

    #[test]
    fn predicate_captures_values_eagerly() {
        let mut ctx = MockCtx::new(true);
        ctx.captures.insert("minYear".into(), SwiftValue::int(1999));
        let (sql, params) = compile_predicate(&mut ctx, "m.year >= minYear");
        assert_eq!(sql, "\"year\" >= ?");
        assert_eq!(params, vec![DbValue::Int(1999)]);
    }

    #[test]
    fn predicate_records_its_generic_model_type() {
        let mut ctx = MockCtx::new(true);
        let node = predicate_node("m.year > 2000");
        let value = predicate_macro(&mut ctx, &node).unwrap();
        let SwiftValue::Object(o) = value else {
            panic!()
        };
        assert_eq!(
            o.borrow()
                .get("__type")
                .and_then(|v| as_string(&v))
                .as_deref(),
            Some("Movie")
        );
    }

    #[test]
    fn unsupported_predicate_shape_is_a_clear_error() {
        let mut ctx = MockCtx::new(true);
        // Comparing two stored properties is not expressible as a bound param.
        let node = predicate_node("m.year > m.other");
        let err = predicate_macro(&mut ctx, &node).unwrap_err();
        assert!(matches!(err, StdError::Error(_)));
        // An unsupported operator (`~=`) is rejected too.
        let node = predicate_node("m.year + 1 > 2000");
        assert!(predicate_macro(&mut ctx, &node).is_err());
    }

    #[test]
    fn sort_descriptor_reads_keypath_and_order() {
        let mut ctx = MockCtx::new(true);
        let forward = sort_descriptor_init(
            &mut ctx,
            vec![Arg::positional(SwiftValue::Str("kp:year".into()))],
        )
        .unwrap();
        let SwiftValue::Object(o) = &forward else {
            panic!()
        };
        assert_eq!(
            o.borrow().get("column").and_then(as_string).as_deref(),
            Some("year")
        );
        assert_eq!(
            o.borrow().get("order").and_then(as_string).as_deref(),
            Some("forward")
        );

        let reverse = sort_descriptor_init(
            &mut ctx,
            vec![
                Arg::positional(SwiftValue::Str("kp:title".into())),
                labeled(
                    "order",
                    SwiftValue::Enum(Rc::new(tswift_core::EnumObj {
                        type_name: "SortOrder".into(),
                        case: "reverse".into(),
                        payload: vec![],
                    })),
                ),
            ],
        )
        .unwrap();
        let SwiftValue::Object(o) = &reverse else {
            panic!()
        };
        assert_eq!(
            o.borrow().get("order").and_then(as_string).as_deref(),
            Some("reverse")
        );
    }

    #[test]
    fn select_sql_includes_where_order_and_limit() {
        let schema = ModelSchema {
            type_name: "Movie".into(),
            table: "Movie".into(),
            columns: vec![
                Column {
                    name: "title".into(),
                    sql_type: SqlType::Text,
                    not_null: true,
                    is_bool: false,
                },
                Column {
                    name: "year".into(),
                    sql_type: SqlType::Integer,
                    not_null: true,
                    is_bool: false,
                },
            ],
        };
        let plan = FetchPlan {
            where_sql: "\"year\" > ?".into(),
            params: vec![DbValue::Int(2000)],
            type_name: Some("Movie".into()),
            order_by: vec![("year".into(), true), ("title".into(), false)],
            limit: Some(5),
            offset: 0,
        };
        assert_eq!(
            select_sql(&schema, &plan),
            "SELECT rowid, \"title\", \"year\" FROM \"Movie\" WHERE \"year\" > ? \
             ORDER BY \"year\" DESC, \"title\" ASC LIMIT 5"
        );
    }

    #[test]
    fn select_sql_paginates_with_limit_and_offset() {
        let schema = ModelSchema {
            type_name: "Movie".into(),
            table: "Movie".into(),
            columns: vec![Column {
                name: "title".into(),
                sql_type: SqlType::Text,
                not_null: true,
                is_bool: false,
            }],
        };
        let base = |limit, offset| FetchPlan {
            where_sql: String::new(),
            params: vec![],
            type_name: Some("Movie".into()),
            order_by: vec![],
            limit,
            offset,
        };
        // limit + offset together.
        assert!(select_sql(&schema, &base(Some(5), 10)).ends_with("LIMIT 5 OFFSET 10"));
        // offset alone needs `LIMIT -1` so SQLite accepts the OFFSET.
        assert!(select_sql(&schema, &base(None, 3)).ends_with("LIMIT -1 OFFSET 3"));
        // no pagination clause when neither is set.
        assert!(!select_sql(&schema, &base(None, 0)).contains("LIMIT"));
    }

    /// Build a container and stock a canned query reply of two Movie rows.
    fn movie_rows() -> Vec<DbRow> {
        vec![
            vec![
                ("rowid".into(), DbValue::Int(1)),
                ("title".into(), DbValue::Text("Arrival".into())),
                ("year".into(), DbValue::Int(2016)),
            ],
            vec![
                ("rowid".into(), DbValue::Int(2)),
                ("title".into(), DbValue::Text("Dune".into())),
                ("year".into(), DbValue::Int(2021)),
            ],
        ]
    }

    #[test]
    fn fetch_emits_single_select_and_decodes_rows() {
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        ctx.query_rows.push(("SELECT".into(), movie_rows()));
        let (_c, main) = movie_container(&mut ctx);

        let sort = sort_descriptor_init(
            &mut ctx,
            vec![
                Arg::positional(SwiftValue::Str("kp:year".into())),
                labeled(
                    "order",
                    SwiftValue::Enum(Rc::new(tswift_core::EnumObj {
                        type_name: "SortOrder".into(),
                        case: "reverse".into(),
                        payload: vec![],
                    })),
                ),
            ],
        )
        .unwrap();
        let descriptor = fetch_descriptor_init(
            &mut ctx,
            vec![labeled("sortBy", SwiftValue::Array(Rc::new(vec![sort])))],
        )
        .unwrap();

        let before = ctx.calls.len();
        let out = context_fetch(&mut ctx, main, vec![descriptor]).unwrap();
        // Exactly one query call, with the expected SELECT.
        let queries: Vec<_> = ctx.calls[before..]
            .iter()
            .filter(|c| matches!(c, Call::Query(_, _)))
            .collect();
        assert_eq!(queries.len(), 1);
        assert_eq!(
            queries[0],
            &Call::Query(
                "SELECT rowid, \"title\", \"year\" FROM \"Movie\" ORDER BY \"year\" DESC".into(),
                vec![]
            )
        );
        // Two decoded Movie objects.
        let SwiftValue::Array(items) = out.result else {
            panic!()
        };
        assert_eq!(items.len(), 2);
        let SwiftValue::Object(first) = &items[0] else {
            panic!()
        };
        assert_eq!(first.borrow().class_name, "Movie");
        assert_eq!(
            first
                .borrow()
                .get("title")
                .and_then(|v| as_string(&v))
                .as_deref(),
            Some("Arrival")
        );
    }

    #[test]
    fn fetch_returns_same_instance_for_same_row() {
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        ctx.query_rows.push(("SELECT".into(), movie_rows()));
        let (_c, main) = movie_container(&mut ctx);

        let descriptor = fetch_descriptor_init(&mut ctx, vec![]).unwrap();
        let first = context_fetch(&mut ctx, main.clone(), vec![descriptor.clone()]).unwrap();
        let second = context_fetch(&mut ctx, main, vec![descriptor]).unwrap();
        let (SwiftValue::Array(a), SwiftValue::Array(b)) = (first.result, second.result) else {
            panic!()
        };
        // The identity map returns the *same* Rc across two fetches of row 1.
        let (SwiftValue::Object(oa), SwiftValue::Object(ob)) = (&a[0], &b[0]) else {
            panic!()
        };
        assert!(Rc::ptr_eq(oa, ob));
    }

    #[test]
    fn fetched_object_mutation_then_save_updates_by_rowid() {
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        ctx.query_rows.push(("SELECT".into(), movie_rows()));
        let (_c, main) = movie_container(&mut ctx);
        let descriptor = fetch_descriptor_init(&mut ctx, vec![]).unwrap();
        let fetched = context_fetch(&mut ctx, main.clone(), vec![descriptor]).unwrap();
        let SwiftValue::Array(items) = fetched.result else {
            panic!()
        };
        let SwiftValue::Object(movie) = &items[0] else {
            panic!()
        };
        movie.borrow_mut().set("year", SwiftValue::int(1999));
        let before = ctx.calls.len();
        context_save(&mut ctx, main, vec![]).unwrap();
        assert_eq!(ctx.calls[before], Call::Begin);
        assert_eq!(
            ctx.calls[before + 1],
            Call::Execute(
                "UPDATE \"Movie\" SET \"title\" = ?, \"year\" = ? WHERE rowid = ?".into(),
                vec![
                    DbValue::Text("Arrival".into()),
                    DbValue::Int(1999),
                    DbValue::Int(1)
                ]
            )
        );
        assert_eq!(ctx.calls[before + 2], Call::Commit);
    }

    // ---------------------------------------------------------------------
    // Issue 1: identity map must be keyed by (type, rowid), not rowid alone.
    // ---------------------------------------------------------------------

    /// Build an in-memory container registering *two* `@Model` types sharing one
    /// connection, returning `(container, main_context)`.
    fn two_model_container(ctx: &mut MockCtx) -> SwiftValue {
        let config = model_configuration_init(
            ctx,
            vec![labeled("isStoredInMemoryOnly", SwiftValue::Bool(true))],
        )
        .unwrap();
        let container = model_container_init(
            ctx,
            vec![
                labeled("for", SwiftValue::Metatype("Movie".into())),
                Arg::positional(SwiftValue::Metatype("Actor".into())),
                labeled("configurations", config),
            ],
        )
        .unwrap();
        container_main_context(ctx, container).unwrap()
    }

    fn fetch_one(
        ctx: &mut MockCtx,
        main: &SwiftValue,
        predicate: SwiftValue,
    ) -> Rc<RefCell<ClassObj>> {
        let descriptor = fetch_descriptor_init(ctx, vec![labeled("predicate", predicate)]).unwrap();
        let out = context_fetch(ctx, main.clone(), vec![descriptor]).unwrap();
        let SwiftValue::Array(items) = out.result else {
            panic!("fetch did not return an array")
        };
        let SwiftValue::Object(obj) = &items[0] else {
            panic!("fetched element is not an object")
        };
        Rc::clone(obj)
    }

    #[test]
    fn fetch_across_model_types_with_same_rowid_returns_distinct_instances() {
        // Both tables own a row with rowid == 1 (rowid is unique only per table).
        let mut ctx = MockCtx::new(true)
            .with_model("Movie", &[("year", "Int")])
            .with_model("Actor", &[("name", "String")]);
        ctx.query_rows.push((
            "FROM \"Movie\"".into(),
            vec![vec![
                ("rowid".into(), DbValue::Int(1)),
                ("year".into(), DbValue::Int(2016)),
            ]],
        ));
        ctx.query_rows.push((
            "FROM \"Actor\"".into(),
            vec![vec![
                ("rowid".into(), DbValue::Int(1)),
                ("name".into(), DbValue::Text("Amy".into())),
            ]],
        ));
        let main = two_model_container(&mut ctx);

        let movie_pred = predicate_macro(&mut ctx, &predicate_node("m.year > 0")).unwrap();
        let actor_pred = predicate_macro(
            &mut ctx,
            &typed_predicate_node("Actor", "a", "a.name == \"Amy\""),
        )
        .unwrap();

        // Fetch Movie (rowid 1) then Actor (rowid 1). A rowid-only identity map
        // would return the Movie instance for the Actor fetch.
        let movie = fetch_one(&mut ctx, &main, movie_pred);
        let actor = fetch_one(&mut ctx, &main, actor_pred);

        assert_eq!(movie.borrow().class_name, "Movie");
        assert_eq!(actor.borrow().class_name, "Actor");
        assert!(!Rc::ptr_eq(&movie, &actor), "instances must be distinct");

        // Mutating each and saving must UPDATE the correct table, both rowid 1.
        movie.borrow_mut().set("year", SwiftValue::int(1999));
        actor
            .borrow_mut()
            .set("name", SwiftValue::Str("Amy A".into()));
        let before = ctx.calls.len();
        context_save(&mut ctx, main, vec![]).unwrap();
        let executes: Vec<_> = ctx.calls[before..]
            .iter()
            .filter_map(|c| match c {
                Call::Execute(sql, p) => Some((sql.clone(), p.clone())),
                _ => None,
            })
            .collect();
        assert!(executes.contains(&(
            "UPDATE \"Movie\" SET \"year\" = ? WHERE rowid = ?".into(),
            vec![DbValue::Int(1999), DbValue::Int(1)],
        )));
        assert!(executes.contains(&(
            "UPDATE \"Actor\" SET \"name\" = ? WHERE rowid = ?".into(),
            vec![DbValue::Text("Amy A".into()), DbValue::Int(1)],
        )));
    }

    // ---------------------------------------------------------------------
    // Issue 2: objects pending deletion are excluded from fetch results.
    // ---------------------------------------------------------------------

    #[test]
    fn pending_deleted_object_is_excluded_from_fetch() {
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        // The store still physically holds both rows until save commits.
        ctx.query_rows.push(("SELECT".into(), movie_rows()));
        let (_c, main) = movie_container(&mut ctx);

        // Insert + save one row (rowid 1), then mark it deleted (not yet saved).
        let m = movie("Arrival", 2016);
        context_insert(&mut ctx, main.clone(), vec![m.clone()]).unwrap();
        context_save(&mut ctx, main.clone(), vec![]).unwrap();
        context_delete(&mut ctx, main.clone(), vec![m]).unwrap();

        let descriptor = fetch_descriptor_init(&mut ctx, vec![]).unwrap();
        let out = context_fetch(&mut ctx, main, vec![descriptor]).unwrap();
        let SwiftValue::Array(items) = out.result else {
            panic!()
        };
        // rowid 1 is pending deletion → excluded; only rowid 2 (Dune) remains.
        assert_eq!(items.len(), 1);
        let SwiftValue::Object(o) = &items[0] else {
            panic!()
        };
        assert_eq!(
            o.borrow().get("title").and_then(as_string).as_deref(),
            Some("Dune")
        );
    }

    // ---------------------------------------------------------------------
    // Issue 3: predicate lowering validates property references vs. schema.
    // ---------------------------------------------------------------------

    #[test]
    fn predicate_validates_property_references_against_schema() {
        let mut ctx = MockCtx::new(true).with_model(
            "Movie",
            &[
                ("title", "String"),
                ("year", "Int"),
                ("watched", "Bool"),
                ("body", "String?"),
            ],
        );

        // Bare-bool position requires a Bool property.
        assert!(predicate_macro(&mut ctx, &predicate_node("m.year")).is_err());
        assert!(predicate_macro(&mut ctx, &predicate_node("m.watched")).is_ok());

        // String methods require a String property.
        assert!(predicate_macro(&mut ctx, &predicate_node("m.year.contains(\"x\")")).is_err());
        assert!(predicate_macro(&mut ctx, &predicate_node("m.title.contains(\"x\")")).is_ok());

        // nil comparison requires an optional property.
        assert!(predicate_macro(&mut ctx, &predicate_node("m.title == nil")).is_err());
        assert!(predicate_macro(&mut ctx, &predicate_node("m.body == nil")).is_ok());

        // An unknown property is a clear error too.
        assert!(predicate_macro(&mut ctx, &predicate_node("m.nope > 1")).is_err());
    }

    // ---------------------------------------------------------------------
    // SwiftUI integration: .modelContainer(for:) modifier + @Query context.
    // ---------------------------------------------------------------------

    /// A throwaway view value to hand the `.modelContainer(for:)` modifier as
    /// its receiver. Named so distinct sites (distinct view types) get distinct
    /// in-memory containers.
    fn named_view(type_name: &str) -> SwiftValue {
        SwiftValue::Struct(Rc::new(StructObj {
            type_name: type_name.into(),
            fields: vec![],
        }))
    }

    fn dummy_view() -> SwiftValue {
        named_view("NoteList")
    }

    /// Apply `.modelContainer(for: model, inMemory: true)` to a freshly-named
    /// view and return the modified view (carrying the stashed context field),
    /// as the renderer would see it before entering the subtree.
    fn apply_in_memory_container(ctx: &mut MockCtx, view_type: &str, model: &str) -> SwiftValue {
        modifier_model_container(
            ctx,
            named_view(view_type),
            vec![metatype(model), labeled("inMemory", SwiftValue::Bool(true))],
        )
        .unwrap()
    }

    #[test]
    fn model_container_modifier_publishes_context_and_reuses_across_renders() {
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        let iid = ctx.id;

        // No container in scope yet -> current-context lookup errors.
        assert!(current_model_context(&mut ctx, vec![]).is_err());

        // First render: the modifier opens the store and stashes its context on
        // the modified view; the context is NOT published until the renderer
        // enters that view's subtree.
        let out = apply_in_memory_container(&mut ctx, "NoteList", "Movie");
        assert_eq!(out.type_name(), "NoteList");
        assert!(
            current_model_context(&mut ctx, vec![]).is_err(),
            "context is scoped to the subtree, not published on modifier apply"
        );
        let opens = ctx
            .calls
            .iter()
            .filter(|c| matches!(c, Call::Open(_)))
            .count();
        assert_eq!(opens, 1, "first render opens the store once");

        // Entering the subtree publishes the context; it resolves to a
        // ModelContext. Exiting withdraws it.
        scope_enter(&mut ctx, &out);
        let published = current_model_context(&mut ctx, vec![]).unwrap();
        assert!(object_int_field(&published, "__ctxid").is_some());
        scope_exit(&mut ctx, &out);
        assert!(
            with_state(iid, |s| s.current_context.is_none()),
            "scoped context withdrawn after leaving the subtree"
        );

        // Second render of the same site reuses the container (no new Open),
        // matching "one container per site across renders".
        let before = ctx.calls.len();
        let out2 = apply_in_memory_container(&mut ctx, "NoteList", "Movie");
        let new_opens = ctx.calls[before..]
            .iter()
            .filter(|c| matches!(c, Call::Open(_)))
            .count();
        assert_eq!(new_opens, 0, "re-render reuses the container, no re-open");

        // Same context instance is published on the reuse render's subtree.
        scope_enter(&mut ctx, &out2);
        let again = current_model_context(&mut ctx, vec![]).unwrap();
        assert_eq!(
            object_int_field(&again, "__ctxid"),
            object_int_field(&published, "__ctxid")
        );
        scope_exit(&mut ctx, &out2);

        teardown_registry(&mut ctx);
        assert!(!REGISTRY.with(|r| r.borrow().contains_key(&iid)));
    }

    #[test]
    fn model_container_modifier_requires_a_model_type() {
        let mut ctx = MockCtx::new(true);
        assert!(modifier_model_container(&mut ctx, dummy_view(), vec![]).is_err());
        teardown_registry(&mut ctx);
    }

    #[test]
    fn sibling_in_memory_containers_stay_isolated() {
        // Two sibling `.modelContainer(for: Movie.self, inMemory: true)` on
        // distinct views must own distinct in-memory databases (each modifier
        // instance owns its container) -> two Opens, distinct contexts.
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        let a = apply_in_memory_container(&mut ctx, "PaneA", "Movie");
        let b = apply_in_memory_container(&mut ctx, "PaneB", "Movie");
        let opens = ctx
            .calls
            .iter()
            .filter(|c| matches!(c, Call::Open(_)))
            .count();
        assert_eq!(opens, 2, "distinct sites open distinct in-memory stores");

        scope_enter(&mut ctx, &a);
        let ca = object_int_field(&current_model_context(&mut ctx, vec![]).unwrap(), "__ctxid");
        scope_exit(&mut ctx, &a);
        scope_enter(&mut ctx, &b);
        let cb = object_int_field(&current_model_context(&mut ctx, vec![]).unwrap(), "__ctxid");
        scope_exit(&mut ctx, &b);
        assert_ne!(ca, cb, "sibling containers publish distinct contexts");
        teardown_registry(&mut ctx);
    }

    #[test]
    fn nested_container_nearest_ancestor_wins_and_restores() {
        // Outer subtree publishes context O; a nested `.modelContainer` publishes
        // context I for its inner subtree (nearest wins); leaving the inner
        // subtree restores O; leaving the outer restores "no context".
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        let outer = apply_in_memory_container(&mut ctx, "Outer", "Movie");
        let inner = apply_in_memory_container(&mut ctx, "Inner", "Movie");

        scope_enter(&mut ctx, &outer);
        let o = object_int_field(&current_model_context(&mut ctx, vec![]).unwrap(), "__ctxid");
        scope_enter(&mut ctx, &inner);
        let i = object_int_field(&current_model_context(&mut ctx, vec![]).unwrap(), "__ctxid");
        assert_ne!(o, i, "nested container overrides for its subtree");
        scope_exit(&mut ctx, &inner);
        let back = object_int_field(&current_model_context(&mut ctx, vec![]).unwrap(), "__ctxid");
        assert_eq!(
            back, o,
            "leaving the inner subtree restores the outer context"
        );
        scope_exit(&mut ctx, &outer);
        assert!(
            with_state(ctx.id, |s| s.current_context.is_none()),
            "leaving the outer subtree restores no scoped context"
        );
        teardown_registry(&mut ctx);
    }

    #[test]
    fn unmarked_view_subtree_sees_no_context() {
        // A plain view (no `.modelContainer`) with no ancestor container: its
        // subtree sees no context, so `@Query` gets a clear diagnostic (which it
        // catches with `try?` and degrades to []). Not a stale leak.
        let mut ctx = MockCtx::new(true);
        let plain = named_view("Plain");
        scope_enter(&mut ctx, &plain);
        let err = current_model_context(&mut ctx, vec![]).unwrap_err();
        if let StdError::Throw(v) = err {
            let msg = struct_field(&v, "message")
                .and_then(|m| as_string(&m))
                .unwrap();
            assert!(
                msg.contains("no model container"),
                "clear diagnostic: {msg}"
            );
        } else {
            panic!("expected a catchable throw, got {err:?}");
        }
        scope_exit(&mut ctx, &plain);
        teardown_registry(&mut ctx);
    }

    #[test]
    fn query_fetch_reflects_inserts_after_save() {
        // End-to-end at the native layer: publish a context via the modifier,
        // insert + save, then fetch what `@Query`'s getter fetches (the sole
        // schema) and see the new rows — the render-on-every-dispatch story.
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        // The mock SELECT reply mirrors what a real store returns post-save.
        ctx.query_rows.push((
            "SELECT".into(),
            vec![vec![
                ("rowid".into(), DbValue::Int(1)),
                ("title".into(), DbValue::Text("Arrival".into())),
                ("year".into(), DbValue::Int(2016)),
            ]],
        ));
        let view = apply_in_memory_container(&mut ctx, "NoteList", "Movie");
        scope_enter(&mut ctx, &view);
        let main = current_model_context(&mut ctx, vec![]).unwrap();

        context_insert(&mut ctx, main.clone(), vec![movie("Arrival", 2016)]).unwrap();
        context_save(&mut ctx, main.clone(), vec![]).unwrap();

        // `@Query` (no predicate) fetches the sole schema via a default
        // FetchDescriptor. Assert a single SELECT is emitted per fetch.
        let descriptor = fetch_descriptor_init(&mut ctx, vec![]).unwrap();
        let before = ctx.calls.len();
        let rows = context_fetch(&mut ctx, main.clone(), vec![descriptor]).unwrap();
        let selects = ctx.calls[before..]
            .iter()
            .filter(|c| matches!(c, Call::Query(_, _)))
            .count();
        assert_eq!(selects, 1, "one SELECT per @Query fetch");
        if let SwiftValue::Array(items) = rows.result {
            assert_eq!(items.len(), 1, "the saved row is fetched back");
        } else {
            panic!("fetch did not return an array");
        }
        scope_exit(&mut ctx, &view);
        teardown_registry(&mut ctx);
    }

    #[test]
    fn action_context_cleared_when_container_removed_next_render() {
        // A container present in render 1 but removed in render 2 (a conditional
        // view) must not leave a stale `action_context`: an out-of-render action
        // after render 2 gets the clean no-container diagnostic, never a write
        // through the removed container.
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);
        let iid = ctx.id;
        let root = named_view("App");

        // Render 1: the root's subtree contains a `.modelContainer(for:)`.
        let marked = apply_in_memory_container(&mut ctx, "NoteList", "Movie");
        scope_enter(&mut ctx, &root);
        scope_enter(&mut ctx, &marked);
        scope_exit(&mut ctx, &marked);
        scope_exit(&mut ctx, &root);

        // Out-of-render action after render 1 resolves the container's context.
        assert!(
            current_model_context(&mut ctx, vec![]).is_ok(),
            "action after a render with a container resolves it"
        );

        // Render 2: the conditional dropped the container; only the plain root
        // renders this pass.
        scope_enter(&mut ctx, &root);
        scope_exit(&mut ctx, &root);

        // Out-of-render action after render 2: clean diagnostic, not a stale write.
        let err = current_model_context(&mut ctx, vec![]).unwrap_err();
        match err {
            StdError::Throw(v) => {
                let msg = struct_field(&v, "message")
                    .and_then(|m| as_string(&m))
                    .unwrap();
                assert!(
                    msg.contains("no model container"),
                    "clean diagnostic: {msg}"
                );
            }
            other => panic!("expected a catchable throw, got {other:?}"),
        }
        assert!(
            with_state(iid, |s| s.action_context.is_none()),
            "stale action context cleared once its container vanished"
        );
        teardown_registry(&mut ctx);
    }

    #[test]
    fn default_store_key_normalized_so_explicit_name_reuses_container() {
        // The default persistent store is keyed by an empty name but opens
        // `default.store`. An explicit `ModelConfiguration("default.store")`
        // names the *same* file, so it must reuse the implicit-default
        // container rather than opening a second one over the same store.
        let mut ctx =
            MockCtx::new(true).with_model("Movie", &[("title", "String"), ("year", "Int")]);

        // Site A: implicit default store (no ModelConfiguration name).
        modifier_model_container(&mut ctx, named_view("PaneA"), vec![metatype("Movie")]).unwrap();

        // Site B: explicit ModelConfiguration("default.store") over the same file.
        let config = model_configuration_init(
            &mut ctx,
            vec![Arg::positional(SwiftValue::Str("default.store".into()))],
        )
        .unwrap();
        modifier_model_container(
            &mut ctx,
            named_view("PaneB"),
            vec![metatype("Movie"), labeled("configurations", config)],
        )
        .unwrap();

        let opens = ctx
            .calls
            .iter()
            .filter(|c| matches!(c, Call::Open(_)))
            .count();
        assert_eq!(
            opens, 1,
            "explicit default.store reuses the implicit-default container (same file)"
        );
        teardown_registry(&mut ctx);
    }
}
