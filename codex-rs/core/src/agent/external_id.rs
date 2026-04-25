use codex_protocol::ThreadId;
use sha2::Digest;
use sha2::Sha256;

pub(crate) fn claude_agent_external_id(thread_id: ThreadId) -> String {
    let digest = Sha256::digest(thread_id.to_string().as_bytes());
    let bytes: [u8; 8] = digest[..8]
        .try_into()
        .expect("sha256 digest prefix should fit into eight bytes");
    format!("a{:016x}", u64::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn claude_agent_external_id_is_stable_and_short() {
        let thread_id = ThreadId::from_string("019daf30-9a44-7943-87ed-0f2c652d330b")
            .expect("thread id should parse");
        let external_id = claude_agent_external_id(thread_id);

        assert_eq!(external_id.len(), 17);
        assert_eq!(external_id.chars().next(), Some('a'));
        assert_eq!(external_id, claude_agent_external_id(thread_id));
    }
}
