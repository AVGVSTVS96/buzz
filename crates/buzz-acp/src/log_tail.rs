//! Recent harness log lines, kept in memory so the owner can read a remote
//! agent's log over the relay observer: a `log_follow` control frame asks for
//! a tail, and `log` telemetry events stream new lines while the request's
//! lease lasts. Nothing is published unless the owner asks.

use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use tokio::{sync::mpsc, time::Instant};
use tracing_subscriber::{filter::LevelFilter, fmt::MakeWriter, registry::LookupSpan, Layer};

use crate::observer::{ObserverContext, ObserverHandle};

/// Lines kept for tails, which is also the largest tail an owner can request.
const LOG_TAIL_CAPACITY: usize = 1_000;
const LOG_LINE_MAX_BYTES: usize = 4_096;
/// Serialized bytes of lines per `log` event. The headroom covers the observer
/// envelope under the NIP-44 plaintext cap, which nostr enforces at 65,408
/// bytes, below `OBSERVER_MAX_PLAINTEXT_LEN`.
const LOG_FRAME_MAX_BYTES: usize = 60_000;
const LOG_FOLLOW_LEASE: Duration = Duration::from_secs(60);
const LOG_STREAM_TICK: Duration = Duration::from_secs(1);

/// Bounded ring of formatted log lines, filled by [`LogTail::layer`].
#[derive(Clone, Default)]
pub(crate) struct LogTail(Arc<Mutex<LogRing>>);

#[derive(Default)]
struct LogRing {
    lines: VecDeque<String>,
    /// Position the next pushed line takes; the oldest retained line sits at
    /// `next - lines.len()`.
    next: u64,
}

/// Lines read from the ring for one `log` event.
struct LogBatch {
    lines: Vec<String>,
    dropped: u64,
    next: u64,
}

impl LogTail {
    /// Formats events like the stderr log, without ANSI colors, into the ring.
    pub(crate) fn layer<S>(&self) -> impl Layer<S>
    where
        S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    {
        tracing_subscriber::fmt::layer()
            .with_writer(self.clone())
            .with_ansi(false)
            .compact()
            // DEBUG includes the relay's per-event OK lines, so streaming it
            // would make every published log frame produce the next one.
            .with_filter(LevelFilter::INFO)
    }

    /// Start answering `log_follow` requests by emitting `log` events into
    /// `observer`. The streaming task ends when every [`LogFollows`] is dropped.
    pub(crate) fn follow(&self, observer: ObserverHandle) -> LogFollows {
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(stream_follows(self.clone(), observer, rx));
        LogFollows(tx)
    }

    fn ring(&self) -> MutexGuard<'_, LogRing> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn push(&self, record: &str) {
        let record = record.trim_end_matches('\n');
        let line = &record[..crate::floor_char_boundary(record, LOG_LINE_MAX_BYTES)];
        let mut ring = self.ring();
        if ring.lines.len() == LOG_TAIL_CAPACITY {
            ring.lines.pop_front();
        }
        ring.lines.push_back(line.to_string());
        ring.next += 1;
    }

    /// The newest `count` lines that fit one event.
    fn newest(&self, count: usize) -> LogBatch {
        let (mut lines, next) = {
            let ring = self.ring();
            let skip = ring.lines.len().saturating_sub(count);
            (ring.lines.iter().skip(skip).cloned().collect(), ring.next)
        };
        keep_newest_fitting(&mut lines);
        LogBatch {
            lines,
            dropped: 0,
            next,
        }
    }

    /// Lines pushed since `cursor`, counting those evicted before they were
    /// read or cut to fit one event as dropped.
    fn since(&self, cursor: u64) -> LogBatch {
        let (mut lines, evicted, next) = {
            let ring = self.ring();
            let oldest = ring.next - ring.lines.len() as u64;
            let skip = cursor.saturating_sub(oldest) as usize;
            (
                ring.lines.iter().skip(skip).cloned().collect(),
                oldest.saturating_sub(cursor),
                ring.next,
            )
        };
        let cut = keep_newest_fitting(&mut lines);
        LogBatch {
            lines,
            dropped: evicted + cut,
            next,
        }
    }
}

impl<'a> MakeWriter<'a> for LogTail {
    type Writer = &'a LogTail;

    fn make_writer(&'a self) -> Self::Writer {
        self
    }
}

impl io::Write for &LogTail {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.push(&String::from_utf8_lossy(buf));
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Hands owner `log_follow` requests to the streaming task.
#[derive(Clone)]
pub(crate) struct LogFollows(mpsc::Sender<usize>);

impl LogFollows {
    pub(crate) fn request(&self, tail: u64) {
        let tail = tail.min(LOG_TAIL_CAPACITY as u64) as usize;
        if self.0.try_send(tail).is_err() {
            tracing::warn!("dropping log_follow request: log streaming is busy or stopped");
        }
    }
}

async fn stream_follows(
    tail: LogTail,
    observer: ObserverHandle,
    mut follows: mpsc::Receiver<usize>,
) {
    let mut cursor = None;
    let mut lease_end = Instant::now();
    let mut tick = tokio::time::interval(LOG_STREAM_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            follow = follows.recv() => {
                let Some(count) = follow else {
                    return;
                };
                lease_end = Instant::now() + LOG_FOLLOW_LEASE;
                let batch = match cursor {
                    Some(from) if count == 0 => tail.since(from),
                    _ => tail.newest(count),
                };
                cursor = Some(batch.next);
                emit_log(&observer, batch);
            }
            _ = tick.tick(), if cursor.is_some() => {
                let Some(from) = cursor else {
                    continue;
                };
                if Instant::now() >= lease_end {
                    cursor = None;
                    continue;
                }
                let batch = tail.since(from);
                cursor = Some(batch.next);
                if !batch.lines.is_empty() || batch.dropped > 0 {
                    emit_log(&observer, batch);
                }
            }
        }
    }
}

fn emit_log(observer: &ObserverHandle, batch: LogBatch) {
    observer.emit(
        "log",
        None,
        &ObserverContext::default(),
        serde_json::json!({ "lines": batch.lines, "dropped": batch.dropped }),
    );
}

/// Drop the oldest lines until the rest fit one event; returns how many went.
fn keep_newest_fitting(lines: &mut Vec<String>) -> u64 {
    let mut bytes = 0;
    let fitting = lines
        .iter()
        .rev()
        .take_while(|line| {
            bytes += serde_json::to_string(line).map_or(line.len(), |json| json.len()) + 1;
            bytes <= LOG_FRAME_MAX_BYTES
        })
        .count();
    let cut = lines.len() - fitting;
    lines.drain(..cut);
    cut as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::observer::{
        decrypt_observer_payload, encrypt_observer_payload, OBSERVER_FRAME_CONTROL,
    };
    use nostr::Keys;
    use serde_json::{json, Value};
    use tokio::{sync::broadcast, time::timeout};
    use tracing_subscriber::layer::SubscriberExt;

    use crate::observer::ObserverEvent;

    async fn next_log(rx: &mut broadcast::Receiver<ObserverEvent>) -> Value {
        let event = timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("a log event")
            .expect("observer open");
        assert_eq!(event.kind, "log");
        assert_eq!(event.agent_index, None);
        assert_eq!(event.channel_id, None);
        event.payload
    }

    async fn assert_no_event(rx: &mut broadcast::Receiver<ObserverEvent>) {
        assert!(timeout(Duration::from_secs(5), rx.recv()).await.is_err());
    }

    #[test]
    fn layer_keeps_info_lines_as_plain_text_and_skips_debug() {
        let tail = LogTail::default();
        let subscriber = tracing_subscriber::registry().with(tail.layer());
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(channel = 7, "subscribed to channel");
            tracing::debug!("OK for event abc: accepted=true message=");
            tracing::warn!("{}", "é".repeat(3_000));
        });

        let lines = tail.newest(LOG_TAIL_CAPACITY).lines;
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].contains(" INFO "), "{}", lines[0]);
        assert!(
            lines[0].ends_with("subscribed to channel channel=7"),
            "{}",
            lines[0]
        );
        assert!(lines.iter().all(|line| !line.contains('\u{1b}')));
        assert!(lines.iter().all(|line| !line.ends_with('\n')));
        assert!((LOG_LINE_MAX_BYTES - 1..=LOG_LINE_MAX_BYTES).contains(&lines[1].len()));
        assert!(lines[1].ends_with('é'));
    }

    #[test]
    fn ring_keeps_the_newest_lines_and_counts_evicted_ones() {
        let tail = LogTail::default();
        for index in 0..LOG_TAIL_CAPACITY + 5 {
            tail.push(&format!("line {index}\n"));
        }

        let newest = tail.newest(2 * LOG_TAIL_CAPACITY);
        assert_eq!(newest.lines.len(), LOG_TAIL_CAPACITY);
        assert_eq!(newest.lines[0], "line 5");
        assert_eq!(newest.dropped, 0);

        let behind = tail.since(0);
        assert_eq!(behind.dropped, 5);
        assert_eq!(behind.lines.first().map(String::as_str), Some("line 5"));

        let caught_up = tail.since(behind.next);
        assert!(caught_up.lines.is_empty());
        assert_eq!(caught_up.dropped, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn follow_answers_streams_new_lines_and_lapses_without_renewal() {
        let tail = LogTail::default();
        let observer = ObserverHandle::in_process();
        let mut rx = observer.subscribe();
        let follows = tail.follow(observer.clone());

        follows.request(5);
        assert_eq!(next_log(&mut rx).await, json!({"lines": [], "dropped": 0}));

        tail.push("first\n");
        tail.push("second\n");
        assert_eq!(next_log(&mut rx).await["lines"], json!(["first", "second"]));
        assert_no_event(&mut rx).await;

        tokio::time::sleep(Duration::from_secs(45)).await;
        follows.request(0);
        assert_eq!(next_log(&mut rx).await["lines"], json!([]));
        tokio::time::sleep(Duration::from_secs(30)).await;
        tail.push("after renewal");
        assert_eq!(next_log(&mut rx).await["lines"], json!(["after renewal"]));

        tokio::time::sleep(LOG_FOLLOW_LEASE).await;
        tail.push("nobody is watching");
        assert_no_event(&mut rx).await;

        follows.request(0);
        assert_eq!(next_log(&mut rx).await["lines"], json!([]));
    }

    #[tokio::test(start_paused = true)]
    async fn only_the_owner_can_start_a_follow() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let stranger = Keys::generate();
        let tail = LogTail::default();
        tail.push("connected to relay");
        let observer = ObserverHandle::in_process();
        let mut rx = observer.subscribe();
        let follows = tail.follow(observer.clone());
        let (publisher, _published) = crate::RelayEventPublisher::test_pair();
        let mut pool = crate::AgentPool::from_slots(vec![]);

        let follow_from = |sender: &Keys| {
            let request = json!({"type": "log_follow", "requestId": "r1", "tail": 10});
            let encrypted =
                encrypt_observer_payload(sender, &agent.public_key(), &request).expect("encrypt");
            buzz_sdk::build_agent_observer_frame(
                &agent.public_key().to_hex(),
                &agent.public_key().to_hex(),
                OBSERVER_FRAME_CONTROL,
                &encrypted,
            )
            .expect("control frame")
            .sign_with_keys(sender)
            .expect("sign")
        };

        let mut handle = |sender: &Keys| {
            crate::handle_relay_observer_control_event(
                &agent,
                follow_from(sender),
                &mut pool,
                Some(&observer),
                &owner.public_key().to_hex(),
                publisher.clone(),
                Some(&follows),
            )
        };

        handle(&stranger);
        assert_no_event(&mut rx).await;
        handle(&owner);
        assert_eq!(
            next_log(&mut rx).await["lines"],
            json!(["connected to relay"])
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_full_tail_is_published_as_one_encryptable_frame() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let tail = LogTail::default();
        for index in 0..LOG_TAIL_CAPACITY {
            tail.push(&format!("{index:04} {}", "\"quoted\" ".repeat(500)));
        }
        let observer = ObserverHandle::in_process();
        let (publisher, mut published) = crate::RelayEventPublisher::test_pair();
        let _publishing = crate::spawn_relay_observer_publisher(
            observer.clone(),
            publisher,
            agent.clone(),
            agent.public_key().to_hex(),
            owner.public_key().to_hex(),
            owner.public_key(),
        );

        tail.follow(observer).request(LOG_TAIL_CAPACITY as u64);

        let frame = timeout(Duration::from_secs(5), published.recv())
            .await
            .expect("a published frame")
            .expect("publisher open");
        let event: Value = decrypt_observer_payload(&owner, &frame).expect("decrypt");
        assert_eq!(event["kind"], "log");
        let lines = event["payload"]["lines"].as_array().expect("lines");
        assert!(
            lines.len() > 1 && lines.len() < LOG_TAIL_CAPACITY,
            "{}",
            lines.len()
        );
        assert!(lines
            .last()
            .and_then(Value::as_str)
            .is_some_and(|line| line.starts_with("0999 ")));
        assert_eq!(event["payload"]["dropped"], 0);
    }
}
