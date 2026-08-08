//! Searching entries across a distribution.
use crate::idb::Entry;
use crate::names::wildcard_match;

/// A search query.
///
/// Currently supports shell-style wildcard matching against normalized
/// entry paths, e.g. `Query::path("*ge7.bin")`.
#[derive(Debug, Clone, Default)]
pub struct Query {
    path_pattern: Option<String>,
}

impl Query {
    /// A query matching entry paths against a wildcard pattern.
    pub fn path(pattern: impl Into<String>) -> Self {
        Query {
            path_pattern: Some(pattern.into()),
        }
    }

    /// Whether an entry matches this query.
    pub(crate) fn matches(&self, entry: &Entry) -> bool {
        match &self.path_pattern {
            Some(pattern) => wildcard_match(pattern, entry.path.as_str()),
            None => true,
        }
    }
}

/// The result of running a [`Query`].
#[derive(Debug, Default)]
pub struct QueryResult<'a> {
    /// Matching entries, in per-product IDB order.
    pub entries: Vec<&'a Entry>,
}
