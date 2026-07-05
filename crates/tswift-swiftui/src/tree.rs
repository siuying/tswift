//! Shared structural-id tree walks over UIIR view-value trees.
//!
//! Every walk over a rendered view tree must assign node ids exactly the way
//! the serializer does (`uiir::write_node`): the root is `"0"` and each child
//! appends one segment via [`crate::child_id`] (positional index, or the row
//! key for keyed `ForEach` children). Events route by these ids, so a walker
//! with a divergent id step silently targets the wrong node. This module
//! centralizes that invariant: [`find`]/[`find_with_ancestor`] (locate the
//! node at a target id), [`walk`] (visit every node), and [`rewrite`]
//! (clone-and-transform). `uiir::write_node` keeps its own fold (it
//! interleaves string output) but shares the same `child_id` step;
//! `diff::diff_node` is a paired two-tree reconciliation and stays separate.

use std::rc::Rc;

use tswift_core::{EvalError, StructObj, SwiftValue};

use crate::{child_id, CHILDREN_FIELD};

/// Find the node at structural path `target`, descending only into subtrees
/// whose id could prefix `target`.
pub(crate) fn find<'a>(tree: &'a SwiftValue, target: &str) -> Option<&'a SwiftValue> {
    find_with_ancestor(tree, target, |_| false).map(|(node, _)| node)
}

/// Like [`find`], but also report the id of the nearest node on the root →
/// target path (target inclusive) matching `pred`, or `None` when no node on
/// the path matches. The candidate is threaded down the recursion by value so
/// a match in an unrelated subtree can never leak into the result.
pub(crate) fn find_with_ancestor<'a>(
    tree: &'a SwiftValue,
    target: &str,
    pred: impl Fn(&StructObj) -> bool,
) -> Option<(&'a SwiftValue, Option<String>)> {
    fn go<'a>(
        node: &'a SwiftValue,
        id: &str,
        target: &str,
        pred: &dyn Fn(&StructObj) -> bool,
        nearest: Option<&str>,
    ) -> Option<(&'a SwiftValue, Option<String>)> {
        let nearest = match node {
            SwiftValue::Struct(obj) if pred(obj) => Some(id),
            _ => nearest,
        };
        if id == target {
            return Some((node, nearest.map(str::to_string)));
        }
        let SwiftValue::Struct(obj) = node else {
            return None;
        };
        // Only descend when `target` lies under this node's subtree.
        let prefix = format!("{id}.");
        if !target.starts_with(&prefix) {
            return None;
        }
        if let Some(SwiftValue::Array(children)) = obj.get(CHILDREN_FIELD) {
            for (i, child) in children.iter().enumerate() {
                if let Some(found) = go(child, &child_id(id, i, child), target, pred, nearest) {
                    return Some(found);
                }
            }
        }
        None
    }
    go(tree, "0", target, &pred, None)
}

/// Visit every struct node in `tree` pre-order as `f(id, depth, node)`, with
/// the root at id `"0"` / depth 0. Depth is threaded explicitly (never parse
/// it from the id — a keyed segment may itself contain dots).
pub(crate) fn walk(tree: &SwiftValue, f: &mut impl FnMut(&str, usize, &StructObj)) {
    fn go(node: &SwiftValue, id: &str, depth: usize, f: &mut dyn FnMut(&str, usize, &StructObj)) {
        let SwiftValue::Struct(obj) = node else {
            return;
        };
        f(id, depth, obj);
        if let Some(SwiftValue::Array(children)) = obj.get(CHILDREN_FIELD) {
            for (i, child) in children.iter().enumerate() {
                go(child, &child_id(id, i, child), depth + 1, f);
            }
        }
    }
    go(tree, "0", 0, f);
}

/// Clone `tree` (rooted at structural path `id`) applying `f` to every struct
/// node post-order: children are rewritten first (with their structural ids),
/// then `f` may mutate the node in place — including replacing or appending
/// children, which are *not* re-walked. Non-struct values pass through
/// unchanged.
pub(crate) fn rewrite(
    node: SwiftValue,
    id: &str,
    f: &mut impl FnMut(&mut StructObj, &str) -> Result<(), EvalError>,
) -> Result<SwiftValue, EvalError> {
    fn go(
        node: SwiftValue,
        id: &str,
        f: &mut dyn FnMut(&mut StructObj, &str) -> Result<(), EvalError>,
    ) -> Result<SwiftValue, EvalError> {
        let SwiftValue::Struct(obj) = node else {
            return Ok(node);
        };
        let mut obj = (*obj).clone();
        if let Some(pos) = obj.fields.iter().position(|(k, _)| k == CHILDREN_FIELD) {
            if let SwiftValue::Array(children) = &obj.fields[pos].1 {
                let kids: Vec<SwiftValue> = children.iter().cloned().collect();
                let mut mapped = Vec::with_capacity(kids.len());
                for (i, c) in kids.into_iter().enumerate() {
                    let cid = child_id(id, i, &c);
                    mapped.push(go(c, &cid, f)?);
                }
                obj.fields[pos].1 = SwiftValue::Array(Rc::new(mapped));
            }
        }
        f(&mut obj, id)?;
        Ok(SwiftValue::Struct(Rc::new(obj)))
    }
    go(node, id, f)
}
