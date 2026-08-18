#![no_main]

use astiango::{index::Index, query::Query, searcher::SearchQuery};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|query: &str| {
    let index = Index::open("/tmp/astiango/fuzz-index").unwrap();

    let ctx = index.inverted_index.local_search_ctx();

    let _ = Query::parse(
        &ctx,
        &SearchQuery {
            query: query.to_string(),
            ..Default::default()
        },
        &index.inverted_index,
    );
});
