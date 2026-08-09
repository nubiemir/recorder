//! Small helpers shared across the crate: stanza lookup and directory setup.

use std::fs::DirBuilder;

use libstrophe::Stanza;
use log::error;

/// Collects every descendant matching `selector`, a `>`-separated path such as
/// `"jingle>content>description"`.
///
/// The path is resolved against each *child* of `element`, not `element`
/// itself, so the first segment names a grandchild of the stanza passed in.
/// Returns an empty vector when `element` is `None` or nothing matches.
pub fn find_all(element: Option<&Stanza>, selector: &str) -> Vec<Stanza> {
    let mut result = vec![];
    let selector: Vec<&str> = selector.split(">").collect();
    if let Some(ele) = element {
        for c in ele.children() {
            let ele = c.get_child_by_path(&selector);
            match ele {
                Some(stan) => {
                    result.push(stan.clone());
                }
                None => {
                    continue;
                }
            }
        }
    }
    result
}

/// Like [`find_all`], but stops at the first match.
pub fn find_first(element: Option<&Stanza>, selector: &str) -> Option<Stanza> {
    let selector: Vec<&str> = selector.split(">").collect();

    if let Some(ele) = element {
        for c in ele.children() {
            if let Some(stan) = c.get_child_by_path(&selector) {
                return Some(stan.clone());
            }
        }
        None
    } else {
        None
    }
}

/// True when at least one descendant matches `selector`.
pub fn exists(element: Option<&Stanza>, selector: &str) -> bool {
    find_all(element, selector).len() != 0
}

/// Creates `output_path` and any missing parents.
///
/// Failure is logged rather than returned: a missing output directory surfaces
/// again as a filesink error on the pipeline bus, and callers sit on hot paths
/// (pad-added, presence) where there is nothing useful to do with the error.
pub fn dir_builder(output_path: &str) {
    match DirBuilder::new().recursive(true).create(&output_path) {
        Err(err) => {
            error!(
                "failed to create directory for: {} err: {err:?}",
                output_path
            );
        }
        _ => {}
    }
}
