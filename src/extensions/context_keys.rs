//! Session-local extension context values, committed before setContext resolves.
use super::*;
const MAX_KEYS: usize = 1024;
const MAX_BYTES: usize = 256 * 1024;
const MAX_VALUE_BYTES: usize = 64 * 1024;
#[derive(Default)]
pub(crate) struct Keys {
    values: HashMap<String, Value>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    key: String,
    remove: bool,
    value: Value,
}
impl Keys {
    pub(crate) fn values(&self) -> &HashMap<String, Value> {
        &self.values
    }
    fn apply(&mut self, value: Value) -> Result<()> {
        let patch: Patch = serde_json::from_value(value)?;
        if patch.key.len() > 1024 {
            bail!("Context key exceeds 1024 UTF-8 bytes");
        }
        let mut remaining = vec![(&patch.value, 0)];
        let mut nodes = 0;
        while let Some((value, depth)) = remaining.pop() {
            nodes += 1;
            if nodes > 16384 || depth > 32 {
                bail!("Context value exceeds depth/node bounds");
            }
            match value {
                Value::Object(value) => remaining.extend(value.values().map(|v| (v, depth + 1))),
                Value::Array(value) => remaining.extend(value.iter().map(|v| (v, depth + 1))),
                _ => {}
            }
        }
        if serde_json::to_vec(&patch.value)?.len() > MAX_VALUE_BYTES {
            bail!("Context value exceeds 64 KiB");
        }
        let mut candidate = self.values.clone();
        if patch.remove {
            candidate.remove(&patch.key);
        } else {
            candidate.insert(patch.key, patch.value);
        }
        if candidate.len() > MAX_KEYS || serde_json::to_vec(&candidate)?.len() > MAX_BYTES {
            bail!("Extension context exceeds 1024 keys / 256 KiB");
        }
        self.values = candidate;
        Ok(())
    }
}
impl Client {
    pub(crate) fn set_context_request(&mut self, params: Value) -> Result<()> {
        if serde_json::to_vec(&params)?.len() > MAX_VALUE_BYTES + 4096 {
            bail!("Context command payload exceeds bounds");
        }
        let request: services::Request = serde_json::from_value(params)?;
        self.validate_native_origin(
            request.session,
            &request.owner,
            request.command,
            request.command_owner.as_deref(),
        )?;
        self.context_keys.apply(request.args)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn values_false_zero_null_removal_and_failed_patch_preserve_original_state() {
        let mut keys = Keys::default();
        for value in [
            json!(false),
            json!(0),
            Value::Null,
            json!(["a", "b"]),
            json!({"a":false}),
        ] {
            keys.apply(json!({"key":"fixture.value","remove":false,"value":value}))
                .unwrap();
            assert_eq!(keys.values()["fixture.value"], value);
        }
        let before = keys.values.clone();
        for patch in [
            json!({"key":"k".repeat(1025),"remove":false,"value":true}),
            json!({"key":"fixture.value","remove":false,"value":"x".repeat(MAX_VALUE_BYTES)}),
            json!({"key":"fixture.value","remove":false,"value":true,"extra":true}),
        ] {
            assert!(keys.apply(patch).is_err());
            assert_eq!(keys.values, before);
        }
        keys.apply(json!({"key":"fixture.value","remove":true,"value":null}))
            .unwrap();
        assert!(keys.values.is_empty());
    }
}

#[cfg(test)]
mod origin_tests {
    use super::*;
    #[test]
    fn outdated_session_and_unknown_owner_cannot_replace_an_accepted_context() {
        let root = tempfile::tempdir().unwrap();
        let extension =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/command-extension");
        let mut client = Client::start(
            "node",
            &extension,
            root.path(),
            &[],
            0,
            &Settings::default(),
        )
        .unwrap();
        let owner = client.packages[0].id.clone();
        let session = client.session;
        let request = |session, owner: &str, value| json!({"session":session,"owner":owner,"generation":0,"args":{"key":"fixture.proof","remove":false,"value":value}});
        client
            .set_context_request(request(session, &owner, true))
            .unwrap();
        assert!(
            client
                .set_context_request(request(session + 1, &owner, false))
                .is_err()
        );
        assert!(
            client
                .set_context_request(request(session, "unknown.owner", false))
                .is_err()
        );
        assert_eq!(client.context_keys.values()["fixture.proof"], true);
    }
}
