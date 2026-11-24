//! Reads numeric error event IDs for one deployment activity.
use std::collections::BTreeSet;
use std::mem::size_of_val;
use windows::Win32::System::EventLog as evt;
use windows::core::GUID;
use windows::core::HSTRING;
use windows::core::Owned;
use windows::core::w;

pub(super) fn failure_event_ids(activity: GUID) -> Option<String> {
    if activity == GUID::zeroed() {
        return None;
    }
    let filter = HSTRING::from(format!(
        "*[System[Correlation[@ActivityID='{{{activity:?}}}'] and Level=2]]"
    ));
    let channel = w!("Microsoft-Windows-AppXDeploymentServer/Operational");
    // Each successful call transfers a handle to us. Owned closes it on every exit.
    unsafe {
        let query =
            Owned::new(evt::EvtQuery(None, channel, &filter, evt::EvtQueryChannelPath.0).ok()?);
        let fields = [w!("Event/System/EventID")];
        let context = Owned::new(
            evt::EvtCreateRenderContext(Some(&fields), evt::EvtRenderContextValues.0).ok()?,
        );
        let mut ids = BTreeSet::new();
        let mut events = [0];
        let mut returned = 0;
        while evt::EvtNext(*query, &mut events, 0, 0, &mut returned).is_ok() && returned == 1 {
            let event = Owned::new(evt::EVT_HANDLE(events[0]));
            let mut value = evt::EVT_VARIANT::default();
            let mut bytes_used = 0;
            let mut properties = 0;
            if evt::EvtRender(
                *context,
                *event,
                evt::EvtRenderEventValues.0,
                size_of_val(&value) as u32,
                Some((&raw mut value).cast()),
                &mut bytes_used,
                &mut properties,
            )
            .is_ok()
                && properties == 1
                && value.Type == evt::EvtVarTypeUInt16.0 as u32
            {
                ids.insert(value.Anonymous.UInt16Val);
            }
        }
        format_failure_event_ids(ids)
    }
}

fn format_failure_event_ids(ids: BTreeSet<u16>) -> Option<String> {
    (!ids.is_empty()).then(|| {
        ids.into_iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_event_ids_are_sorted_unique_and_optional() {
        assert_eq!(failure_event_ids(GUID::zeroed()), None);
        assert_eq!(format_failure_event_ids(BTreeSet::new()), None);
        assert_eq!(
            format_failure_event_ids([404, 100, 404, 5].into_iter().collect()),
            Some("5,100,404".to_string())
        );
    }
}
