use codex_utils_fuzzy_match::fuzzy_match;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FilteredSelectionItem {
    pub(crate) actual_idx: usize,
    pub(crate) match_indices: Option<Vec<usize>>,
}

pub(crate) fn filter_selection_items<'a, I>(query: &str, items: I) -> Vec<FilteredSelectionItem>
where
    I: IntoIterator<Item = (usize, &'a str, Option<&'a str>)>,
{
    let query = query.trim();
    if query.is_empty() {
        return items
            .into_iter()
            .map(|(actual_idx, _, _)| FilteredSelectionItem {
                actual_idx,
                match_indices: None,
            })
            .collect();
    }

    items
        .into_iter()
        .filter_map(|(actual_idx, name, search_value)| {
            if let Some((indices, _score)) = fuzzy_match(name, query) {
                return Some(FilteredSelectionItem {
                    actual_idx,
                    match_indices: Some(indices),
                });
            }

            if let Some(search_value) = search_value.filter(|value| *value != name)
                && fuzzy_match(search_value, query).is_some()
            {
                return Some(FilteredSelectionItem {
                    actual_idx,
                    match_indices: None,
                });
            }

            None
        })
        .collect()
}
