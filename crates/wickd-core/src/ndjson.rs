//! The NDJSON event-line envelope shared by every wickd stream producer.
//!
//! One convention, one implementation (AGT-652): an event is the payload
//! object serialized to a single JSON line with an `"event"` discriminator
//! key inserted. The CLI's stdout sinks (`wickd stream` / `wickd watch`) and
//! the desktop app's hub-hosting sink all render through this function, so a
//! hub client sees byte-identical lines no matter which process owns the hub.

use serde::Serialize;

/// Render `payload` to a single NDJSON line: the payload object with an
/// `"event"` discriminator inserted. Non-object payloads are wrapped as
/// `{"event": ..., "data": ...}`. Returns `None` only if serialization of the
/// assembled value fails (practically never for serde-derived payloads).
pub fn event_line<T: Serialize>(event: &str, payload: &T) -> Option<String> {
    let mut value = serde_json::to_value(payload).unwrap_or(serde_json::Value::Null);
    match value.as_object_mut() {
        Some(obj) => {
            obj.insert(
                "event".to_string(),
                serde_json::Value::String(event.to_string()),
            );
        }
        None => {
            value = serde_json::json!({ "event": event, "data": value });
        }
    }
    serde_json::to_string(&value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_event_discriminator_into_object_payloads() {
        #[derive(Serialize)]
        struct P {
            instrument: &'static str,
        }
        let line = event_line("price-update", &P { instrument: "EUR_USD" }).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["event"], "price-update");
        assert_eq!(v["instrument"], "EUR_USD");
        assert!(!line.contains('\n'));
    }

    #[test]
    fn wraps_non_object_payloads() {
        let line = event_line("weird", &42).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["event"], "weird");
        assert_eq!(v["data"], 42);
    }
}
