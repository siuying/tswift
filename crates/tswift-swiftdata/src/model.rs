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

use crate::db::{self, encode_params, DbValue, ExecResult};

/// The default persistent store name handed to the host when no in-memory
/// configuration is supplied. The host maps it to a real location.
const DEFAULT_STORE: &str = "default.store";

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

    let container = BuiltinReceiver::register_extension("ModelContainer");
    interp.register_contextual_property(container, "mainContext", container_main_context);

    let context = BuiltinReceiver::register_extension("ModelContext");
    for (name, func) in [
        ("insert", context_insert as _),
        ("delete", context_delete as _),
        ("save", context_save as _),
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

fn model_container_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    // Collect every model type (any metatype argument, in any position — the
    // `for:` label leads the variadic) and any ModelConfiguration.
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
    for arg in &args {
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
    for name in &type_names {
        match derive_schema(ctx, name) {
            Ok(schema) => schemas.push(schema),
            Err(err) => return Err(ctx.throw(err)),
        }
    }
    let schemas = Rc::new(schemas);

    let path = if in_memory {
        ":memory:".to_string()
    } else {
        store_name.unwrap_or_else(|| DEFAULT_STORE.to_string())
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
    // Move inserted objects into the tracked set with their new rowid+snapshot.
    let inserted = std::mem::take(&mut state.inserted);
    for (obj, rowid) in inserted.into_iter().zip(insert_rowids) {
        let snapshot = {
            let borrowed = obj.borrow();
            schema_for(&state.schemas, &borrowed.class_name)
                .and_then(|schema| row_values(&borrowed, schema))
                .unwrap_or_default()
        };
        state.tracked.push(Tracked {
            obj,
            rowid,
            snapshot,
        });
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
                other => panic!("unexpected host fn {other}"),
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
                },
                Column {
                    name: "rating".into(),
                    sql_type: SqlType::Real,
                    not_null: false,
                },
                Column {
                    name: "seen".into(),
                    sql_type: SqlType::Integer,
                    not_null: true,
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
}
