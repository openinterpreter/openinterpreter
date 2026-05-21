pub(crate) fn provider_model_label(provider_id: &str, model: &str) -> String {
    let provider_id = provider_id.trim();
    let model = model.trim();

    if model.is_empty() {
        return model.to_string();
    }
    if provider_id.is_empty() {
        return model.to_string();
    }

    format!("{provider_id} {model}")
}
