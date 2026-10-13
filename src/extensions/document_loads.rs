//! Original customer admission before provider callbacks or untitled creation.
use super::*;
use anyhow::ensure;
use std::sync::atomic::{AtomicU8, Ordering};

const LIFETIME: Duration = Duration::from_secs(60);
#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Kind {
    Virtual,
    Untitled,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Begin {
    kind: Kind,
    uri: Option<String>,
}
pub(crate) struct Ticket {
    pub(crate) id: String,
    pub(crate) context: editor_state::Context,
    pub(crate) caller_retired: Arc<std::sync::atomic::AtomicBool>,
    session: u64,
    owner: String,
    kind: Kind,
    uri: Option<String>,
    started: Instant,
    state: AtomicU8,
}
impl Ticket {
    pub(crate) fn retire(&self) {
        self.state.store(3, Ordering::Release);
    }
    pub(crate) fn current(&self, session: u64) -> Result<()> {
        ensure!(
            session == self.session
                && self.started.elapsed() < LIFETIME
                && self.state.load(Ordering::Acquire) < 3,
            "Original document load admission retired or expired"
        );
        Ok(())
    }
    pub(crate) fn issue(&self, session: u64) -> Result<()> {
        self.current(session)?;
        match self
            .state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) | Err(1) => Ok(()),
            _ => anyhow::bail!("Document load admission already consumed"),
        }
    }
}
#[derive(Default)]
pub(super) struct Loads {
    next: u64,
    entries: HashMap<String, Arc<Ticket>>,
}
impl Loads {
    pub(crate) fn begin(
        &mut self,
        session: u64,
        owner: &str,
        context: editor_state::Context,
        caller_retired: Arc<std::sync::atomic::AtomicBool>,
        args: &Value,
    ) -> Result<Arc<Ticket>> {
        self.entries
            .retain(|_, ticket| ticket.current(session).is_ok());
        ensure!(
            self.entries.len() < 8,
            "Customer document admission limit reached"
        );
        let begin: Begin = serde_json::from_value(args.clone())?;
        if let Some(uri) = &begin.uri {
            ensure!(
                !uri.is_empty() && uri.len() <= 4096 && !uri.contains('\0'),
                "Invalid admitted document resource"
            );
            let parsed = url::Url::parse(uri)?;
            ensure!(
                if begin.kind == Kind::Untitled {
                    parsed.scheme() == "untitled"
                } else {
                    !matches!(parsed.scheme(), "file" | "untitled")
                },
                "Document admission kind/resource mismatch"
            );
        } else {
            ensure!(
                begin.kind == Kind::Untitled,
                "Virtual document admission requires a resource"
            );
        }
        self.next = self
            .next
            .checked_add(1)
            .context("Document admission identity exhausted")?;
        let ticket = Arc::new(Ticket {
            id: format!("load-{session}-{}", self.next),
            context,
            caller_retired,
            session,
            owner: owner.into(),
            kind: begin.kind,
            uri: begin.uri,
            started: Instant::now(),
            state: AtomicU8::new(0),
        });
        self.entries.insert(ticket.id.clone(), ticket.clone());
        Ok(ticket)
    }
    pub(crate) fn consume(
        &mut self,
        session: u64,
        owner: &str,
        kind: Option<Kind>,
        args: &mut Value,
    ) -> Result<Arc<Ticket>> {
        let object = args
            .as_object_mut()
            .context("Invalid load completion arguments")?;
        let id = object
            .get("load")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .context("Missing document admission identity")?
            .to_owned();
        let ticket = self
            .entries
            .get(&id)
            .context("Unknown or consumed document admission")?
            .clone();
        ticket.current(session)?;
        ensure!(
            ticket.owner == owner && ticket.state.load(Ordering::Acquire) == 1,
            "Original document admission owner/state mismatch"
        );
        if let Some(kind) = kind {
            ensure!(
                ticket.kind == kind
                    && object
                        .get("uri")
                        .is_some_and(|uri| uri.as_str() == ticket.uri.as_deref()
                            && (uri.is_string() || uri.is_null())),
                "Original document admission resource/kind mismatch"
            );
        } else {
            ensure!(
                object.len() == 1,
                "Invalid document admission cancellation fields"
            );
        }
        self.entries.remove(&id);
        ticket
            .state
            .store(if kind.is_some() { 2 } else { 3 }, Ordering::Release);
        object.remove("load");
        Ok(ticket)
    }
}
impl Drop for Loads {
    fn drop(&mut self) {
        for ticket in self.entries.values() {
            ticket.state.store(3, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn begin(loads: &mut Loads, uri: Option<&str>, kind: &str) -> Arc<Ticket> {
        loads
            .begin(
                7,
                "owner",
                editor_state::Context::new(std::env::temp_dir(), Arc::new(vec![]), 1).unwrap(),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                &json!({"kind":kind,"uri":uri}),
            )
            .unwrap()
    }
    #[test]
    fn one_original_resource_is_issued_then_consumed_once_without_owner_renewal() {
        let mut loads = Loads::default();
        let ticket = begin(&mut loads, Some("preview:/a"), "virtual");
        let original = ticket.started;
        assert!(
            loads
                .consume(
                    7,
                    "owner",
                    Some(Kind::Virtual),
                    &mut json!({"load":ticket.id,"uri":"preview:/a"})
                )
                .is_err()
        );
        ticket.issue(7).unwrap();
        ticket.caller_retired.store(true, Ordering::Release);
        let mut args = json!({"load":ticket.id,"uri":"preview:/a","content":"猫🙂"});
        let claimed = loads
            .consume(7, "owner", Some(Kind::Virtual), &mut args)
            .unwrap();
        assert!(Arc::ptr_eq(&claimed, &ticket));
        assert_eq!(claimed.started, original);
        assert!(args.get("load").is_none());
        assert!(
            loads
                .consume(
                    7,
                    "owner",
                    Some(Kind::Virtual),
                    &mut json!({"load":ticket.id,"uri":"preview:/a"})
                )
                .is_err()
        );
    }
    #[test]
    fn forged_owner_session_resource_and_kind_cannot_consume_a_live_admission() {
        let mut loads = Loads::default();
        let ticket = begin(&mut loads, Some("preview:/a"), "virtual");
        ticket.issue(7).unwrap();
        for (session, owner, kind, uri) in [
            (8, "owner", Kind::Virtual, "preview:/a"),
            (7, "other", Kind::Virtual, "preview:/a"),
            (7, "owner", Kind::Untitled, "preview:/a"),
            (7, "owner", Kind::Virtual, "preview:/b"),
        ] {
            assert!(
                loads
                    .consume(
                        session,
                        owner,
                        Some(kind),
                        &mut json!({"load":ticket.id,"uri":uri})
                    )
                    .is_err()
            );
        }
        assert_eq!(loads.entries.len(), 1);
        loads
            .consume(
                7,
                "owner",
                Some(Kind::Virtual),
                &mut json!({"load":ticket.id,"uri":"preview:/a"}),
            )
            .unwrap();
    }
    #[test]
    fn fresh_untitled_admissions_are_distinct_and_cancel_removes_only_the_original() {
        let mut loads = Loads::default();
        let first = begin(&mut loads, None, "untitled");
        let second = begin(&mut loads, None, "untitled");
        assert_ne!(first.id, second.id);
        first.issue(7).unwrap();
        second.issue(7).unwrap();
        loads
            .consume(7, "owner", None, &mut json!({"load":first.id}))
            .unwrap();
        assert!(first.current(7).is_err());
        assert!(second.current(7).is_ok());
        assert_eq!(loads.entries.len(), 1);
    }
    #[test]
    fn bounded_admissions_and_invalid_shapes_do_not_consume_existing_authority() {
        let mut loads = Loads::default();
        for _ in 0..8 {
            begin(&mut loads, None, "untitled");
        }
        assert!(
            loads
                .begin(
                    7,
                    "owner",
                    editor_state::Context::new(std::env::temp_dir(), Arc::new(vec![]), 1).unwrap(),
                    Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    &json!({"kind":"untitled","uri":null})
                )
                .is_err()
        );
        assert_eq!(loads.entries.len(), 8);
        let ticket = loads.entries.values().next().unwrap().clone();
        ticket.issue(7).unwrap();
        assert!(
            loads
                .consume(
                    7,
                    "owner",
                    None,
                    &mut json!({"load":ticket.id,"extra":true})
                )
                .is_err()
        );
        assert_eq!(loads.entries.len(), 8);
    }
    #[test]
    fn issue_and_consumption_never_renew_the_original_absolute_deadline() {
        let mut loads = Loads::default();
        let mut ticket = begin(&mut loads, Some("preview:/a"), "virtual");
        loads.entries.remove(&ticket.id);
        ticket.issue(7).unwrap();
        Arc::get_mut(&mut ticket).unwrap().started = Instant::now() - LIFETIME;
        assert!(ticket.current(7).is_err());
        assert!(ticket.issue(7).is_err());
        loads.entries.insert(ticket.id.clone(), ticket.clone());
        assert!(
            loads
                .consume(
                    7,
                    "owner",
                    Some(Kind::Virtual),
                    &mut json!({"load":ticket.id,"uri":"preview:/a"})
                )
                .is_err()
        );
    }
}
