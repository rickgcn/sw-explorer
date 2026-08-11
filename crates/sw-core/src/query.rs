//! Searching entries across a distribution.
use crate::distribution::LocatedEntry;
use crate::idb::Entry;
use crate::names::wildcard_match;

/// A search query.
///
/// Supports shell-style wildcard matching against normalized entry
/// paths. [`Query::path`] takes an explicit wildcard pattern;
/// [`Query::path_search`] applies the shared user-input policy (plain
/// text is a substring search) that both frontends use.
#[derive(Debug, Clone, Default)]
pub struct Query {
    path_pattern: Option<String>,
}

/// An error produced when turning user search input into a [`Query`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryError {
    /// The search input was empty: an empty search is never a
    /// "match everything" search.
    Empty,
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryError::Empty => f.write_str("search query is empty"),
        }
    }
}

impl std::error::Error for QueryError {}

impl Query {
    /// A query matching entry paths against a wildcard pattern, e.g.
    /// `Query::path("*ge7.bin")`. The pattern is used exactly as given.
    pub fn path(pattern: impl Into<String>) -> Self {
        Query {
            path_pattern: Some(pattern.into()),
        }
    }

    /// Builds a query from user search input.
    ///
    /// The shared search policy every frontend uses: plain text matches
    /// any path containing it (`libGL` becomes `*libGL*`), while input
    /// carrying its own `*` or `?` is handed to the wildcard matcher
    /// unchanged (`usr/lib/*.so` stays `usr/lib/*.so`).
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Empty`] for empty input; an empty search is
    /// never a "match everything" search.
    pub fn path_search(input: &str) -> Result<Self, QueryError> {
        if input.is_empty() {
            return Err(QueryError::Empty);
        }
        if input.contains(['*', '?']) {
            Ok(Query::path(input))
        } else {
            Ok(Query::path(format!("*{input}*")))
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
    /// Matching entries with their canonical owning-product identity,
    /// in distribution product order, each product in IDB order.
    pub entries: Vec<LocatedEntry<'a>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_a_substring_search() {
        let query = Query::path_search("libGL").unwrap();
        assert_eq!(query.path_pattern.as_deref(), Some("*libGL*"));
    }

    #[test]
    fn explicit_wildcards_are_used_unchanged() {
        let query = Query::path_search("usr/lib/*.so").unwrap();
        assert_eq!(query.path_pattern.as_deref(), Some("usr/lib/*.so"));
        let query = Query::path_search("*Xsgi?").unwrap();
        assert_eq!(query.path_pattern.as_deref(), Some("*Xsgi?"));
    }

    #[test]
    fn empty_input_is_rejected() {
        assert_eq!(Query::path_search("").unwrap_err(), QueryError::Empty);
    }
}
