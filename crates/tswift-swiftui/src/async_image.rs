//! `AsyncImage` — host-side image loading (ADR-0013 §4): the constructor
//! (bare v1 and closure-carrying v1.5 forms) plus the phase-resolution helpers
//! the session uses to realize content/placeholder children per render.

use std::rc::Rc;

use tswift_core::{Arg, StdContext, StdError, StdResult, StructObj, SwiftValue};

use crate::{
    container_value, expand_into, view_value, ASYNC_IMAGE_CONTENT_FIELD, ASYNC_IMAGE_PHASE_FIELD,
    ASYNC_IMAGE_PLACEHOLDER_FIELD, CHILDREN_FIELD,
};

/// `AsyncImage(url:)` — host-side image loading (ADR-0013 §4).
///
/// **v1 bare** (`AsyncImage(url: url)`): serializes as
/// `{"kind":"AsyncImage","args":{"url":".."}}`; no closures, no phase field.
/// The host loads the image natively (`<img src>` / SwiftUI `AsyncImage`).
///
/// **v1.5 content+placeholder** (`AsyncImage(url:) { img in … } placeholder: { … }`):
/// content closure stored in `_asyncContent`; placeholder closure in
/// `_asyncPlaceholder`. A `phase` arg is added (`"empty"` initially); the
/// session evaluates the appropriate closure and sets `_children` on each
/// render via `apply_image_phase`.
///
/// **v1.5 phase** (`AsyncImage(url:) { phase in … }`): single trailing closure
/// stored in `_asyncPhaseContent`, called with an `AsyncImagePhase` struct
/// value. `phase` arg starts `"empty"`; session resolves children the same way.
///
/// Closures are never serialized (leading `_`). Hosts learn the current phase
/// from the serialized `phase` arg and the current children (already resolved).
pub(crate) fn async_image_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut url: Option<String> = None;
    let mut content_closure: Option<usize> = None;
    let mut placeholder_closure: Option<usize> = None;
    let mut trailing_closure: Option<usize> = None;

    for arg in args {
        match arg.label.as_deref() {
            Some("url") => {
                // `URL(string:)` returns a `SwiftValue::Struct` with type_name "URL"
                // and a `_string` field; `nil` on an invalid string.
                if let SwiftValue::Struct(obj) = &arg.value {
                    if obj.type_name == "URL" {
                        if let Some(SwiftValue::Str(s)) = obj.get("_string") {
                            url = Some(s.clone());
                        }
                    }
                }
                // Nil: no URL (loading will never succeed)
            }
            Some("content") => {
                if let SwiftValue::Closure(id) = arg.value {
                    content_closure = Some(id);
                }
            }
            Some("placeholder") => {
                if let SwiftValue::Closure(id) = arg.value {
                    placeholder_closure = Some(id);
                }
            }
            None => {
                if let SwiftValue::Closure(id) = arg.value {
                    trailing_closure = Some(id);
                }
            }
            _ => {}
        }
    }

    let url_str = url.unwrap_or_default();

    // v1 bare — no closures.
    if content_closure.is_none() && placeholder_closure.is_none() && trailing_closure.is_none() {
        return Ok(view_value(
            "AsyncImage",
            vec![("url".into(), SwiftValue::Str(url_str))],
        ));
    }

    // v1.5 — at least one closure present. Disambiguate forms:
    //   - content+placeholder: trailing → content, `placeholder:` → placeholder
    //   - phase: only trailing (no explicit `placeholder:` or `content:`) → phase closure
    let (actual_content, actual_placeholder, actual_phase) =
        if placeholder_closure.is_some() || content_closure.is_some() {
            // content+placeholder form
            let content = content_closure.or(trailing_closure);
            (content, placeholder_closure, None)
        } else {
            // Only a trailing closure → phase form
            (None, None, trailing_closure)
        };

    let mut fields = vec![
        ("url".into(), SwiftValue::Str(url_str)),
        ("phase".into(), SwiftValue::Str("empty".into())),
        (
            CHILDREN_FIELD.into(),
            SwiftValue::Array(Rc::new(Vec::new())),
        ),
    ];
    if let Some(id) = actual_content {
        fields.push((ASYNC_IMAGE_CONTENT_FIELD.into(), SwiftValue::Closure(id)));
    }
    if let Some(id) = actual_placeholder {
        fields.push((
            ASYNC_IMAGE_PLACEHOLDER_FIELD.into(),
            SwiftValue::Closure(id),
        ));
    }
    if let Some(id) = actual_phase {
        fields.push((ASYNC_IMAGE_PHASE_FIELD.into(), SwiftValue::Closure(id)));
    }
    Ok(view_value("AsyncImage", fields))
}

/// Build the `Image` view value passed to an `AsyncImage` content closure on
/// success: a bare `Image` node with a `url` arg so hosts render it as a remote
/// image (`<img src>` / `SwiftUI.AsyncImage`). Content modifiers (`.resizable()`,
/// `.scaledToFit()`) are applied to this value by the closure itself.
pub fn async_image_url_image(url: &str) -> SwiftValue {
    view_value(
        "Image",
        vec![("url".into(), SwiftValue::Str(url.to_string()))],
    )
}

/// Whether `obj` holds any `AsyncImage` closure field (distinguishes v1.5 from
/// bare v1 after deserialization).
pub fn has_async_image_closures(obj: &StructObj) -> bool {
    obj.get(ASYNC_IMAGE_CONTENT_FIELD).is_some()
        || obj.get(ASYNC_IMAGE_PLACEHOLDER_FIELD).is_some()
        || obj.get(ASYNC_IMAGE_PHASE_FIELD).is_some()
}

/// Evaluate an `AsyncImage` node’s closure for `phase` and return the child
/// view to show (ADR-0013 §4):
///
/// - `"success"`: content closure called with `Image(url)`, or `None` when no
///   content closure.
/// - `"empty"` / `"failure"`: placeholder closure called with no args, or
///   `None` when absent.
/// - Phase-form: the single phase closure called with an `AsyncImagePhase`
///   struct value for every phase.
///
/// Returns `None` for bare-v1 nodes (no closures), which need no child
/// injection (the host loads natively).
pub fn realize_async_image_child(
    ctx: &mut dyn StdContext,
    node: &SwiftValue,
    phase: &str,
    url: &str,
) -> Result<Option<SwiftValue>, StdError> {
    let SwiftValue::Struct(obj) = node else {
        return Ok(None);
    };

    // Phase-closure form — pass an AsyncImagePhase struct to the closure.
    if let Some(SwiftValue::Closure(phase_cid)) = obj.get(ASYNC_IMAGE_PHASE_FIELD) {
        let phase_value = make_async_image_phase(phase, url);
        let produced = ctx.eval_block_values_with_args(*phase_cid, vec![phase_value])?;
        return realize_single_async_child(ctx, produced);
    }

    // Content+placeholder form.
    match phase {
        "success" => {
            if let Some(SwiftValue::Closure(cid)) = obj.get(ASYNC_IMAGE_CONTENT_FIELD) {
                let image = async_image_url_image(url);
                let produced = ctx.eval_block_values_with_args(*cid, vec![image])?;
                return realize_single_async_child(ctx, produced);
            }
            Ok(None)
        }
        _ => {
            // "empty" | "failure"
            if let Some(SwiftValue::Closure(cid)) = obj.get(ASYNC_IMAGE_PLACEHOLDER_FIELD) {
                let produced = ctx.eval_block_values(*cid)?;
                return realize_single_async_child(ctx, produced);
            }
            Ok(None)
        }
    }
}

/// Collapse `produced` (the raw value returned by a `@ViewBuilder` block) to a
/// single child view value (or `None` when empty).
fn realize_single_async_child(
    ctx: &mut dyn StdContext,
    produced: SwiftValue,
) -> Result<Option<SwiftValue>, StdError> {
    let mut out = Vec::new();
    expand_into(
        ctx,
        produced,
        &mut out,
        0,
        &crate::EnvironmentContext::default(),
    )?;
    Ok(match out.len() {
        0 => None,
        1 => out.into_iter().next(),
        _ => Some(container_value("Group", out)),
    })
}

/// Build an `AsyncImagePhase` struct value for the phase-closure form.
fn make_async_image_phase(phase: &str, url: &str) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: "AsyncImagePhase".into(),
        fields: vec![
            ("phaseCase".into(), SwiftValue::Str(phase.to_string())),
            ("phaseUrl".into(), SwiftValue::Str(url.to_string())),
        ],
    }))
}
