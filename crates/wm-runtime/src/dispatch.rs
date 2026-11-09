//! Message event dispatch (WM-0314).
//!
//! A behavior subscribes to message ids it cares about; the [`EventDispatcher`]
//! routes an incoming message to every subscribed actor and queues it as a
//! [`BehaviorEvent`] on that actor's pending queue. The dispatcher is pure
//! bookkeeping — it never runs a behavior; the batch runner (WM-0315) drains
//! each actor's queue.

use std::collections::{HashMap, VecDeque};

use crate::behavior::BehaviorEvent;
use crate::lifecycle::ActorId;

/// Routes incoming messages to subscribed actors and holds per-actor event
/// queues.
#[derive(Debug, Default)]
pub struct EventDispatcher {
    subscriptions: HashMap<u32, Vec<ActorId>>,
    pending: HashMap<ActorId, VecDeque<BehaviorEvent>>,
}

impl EventDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe `actor` to `message_id`. Subscribing twice is idempotent.
    pub fn subscribe(&mut self, actor: ActorId, message_id: u32) {
        let subs = self.subscriptions.entry(message_id).or_default();
        if !subs.contains(&actor) {
            subs.push(actor);
        }
    }

    /// Remove `actor`'s subscription to `message_id`.
    pub fn unsubscribe(&mut self, actor: ActorId, message_id: u32) {
        if let Some(subs) = self.subscriptions.get_mut(&message_id) {
            subs.retain(|a| *a != actor);
        }
    }

    /// Number of actors subscribed to `message_id`.
    pub fn subscriber_count(&self, message_id: u32) -> usize {
        self.subscriptions.get(&message_id).map_or(0, Vec::len)
    }

    /// Route one incoming message to every subscriber, queuing an event on
    /// each. Returns the subscribers the event was queued to (the caller wakes
    /// any that are suspended).
    pub fn route_message(&mut self, message_id: u32, payload: &[u8]) -> Vec<ActorId> {
        let subs = match self.subscriptions.get(&message_id) {
            Some(s) => s.clone(),
            None => return Vec::new(),
        };
        for actor in &subs {
            self.pending
                .entry(*actor)
                .or_default()
                .push_back(BehaviorEvent::Message(message_id, payload.to_vec()));
        }
        subs
    }

    /// Queue a timer-fired event directly to one actor.
    pub fn deliver_timer(&mut self, actor: ActorId, timer_id: u64) {
        self.pending
            .entry(actor)
            .or_default()
            .push_back(BehaviorEvent::Timer(timer_id));
    }

    /// Number of events queued for `actor`.
    pub fn pending(&self, actor: ActorId) -> usize {
        self.pending.get(&actor).map_or(0, VecDeque::len)
    }

    /// Pop the next event for `actor`, or `None` when its queue is empty.
    pub fn next_event(&mut self, actor: ActorId) -> Option<BehaviorEvent> {
        let queue = self.pending.get_mut(&actor)?;
        let event = queue.pop_front();
        if queue.is_empty() {
            self.pending.remove(&actor);
        }
        event
    }

    /// Drop all events queued for `actor` (used when it is closed/drained).
    pub fn clear(&mut self, actor: ActorId) {
        self.pending.remove(&actor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aid(n: u64) -> ActorId {
        ActorId(n)
    }

    #[test]
    fn routes_message_to_all_subscribers_in_order() {
        let mut d = EventDispatcher::new();
        d.subscribe(aid(1), 10);
        d.subscribe(aid(2), 10);
        d.subscribe(aid(3), 10);
        assert_eq!(d.subscriber_count(10), 3);
        assert_eq!(
            d.route_message(10, b"payload"),
            vec![aid(1), aid(2), aid(3)]
        );
        assert_eq!(d.pending(aid(1)), 1);
        assert_eq!(
            d.next_event(aid(1)),
            Some(BehaviorEvent::Message(10, b"payload".to_vec()))
        );
        assert_eq!(d.next_event(aid(1)), None);
        assert_eq!(
            d.next_event(aid(2)),
            Some(BehaviorEvent::Message(10, b"payload".to_vec()))
        );
        assert_eq!(
            d.next_event(aid(3)),
            Some(BehaviorEvent::Message(10, b"payload".to_vec()))
        );
    }

    #[test]
    fn no_subscribers_routes_nothing() {
        let mut d = EventDispatcher::new();
        assert!(d.route_message(99, b"x").is_empty());
    }

    #[test]
    fn unsubscribe_stops_delivery() {
        let mut d = EventDispatcher::new();
        d.subscribe(aid(1), 5);
        d.unsubscribe(aid(1), 5);
        assert!(d.route_message(5, b"x").is_empty());
        assert_eq!(d.subscriber_count(5), 0);
    }

    #[test]
    fn duplicate_subscribe_is_idempotent() {
        let mut d = EventDispatcher::new();
        d.subscribe(aid(1), 7);
        d.subscribe(aid(1), 7);
        assert_eq!(d.route_message(7, b"x"), vec![aid(1)], "subscribed once");
    }

    #[test]
    fn timer_and_message_events_share_one_fifo_queue() {
        let mut d = EventDispatcher::new();
        d.subscribe(aid(1), 20);
        d.route_message(20, b"m");
        d.deliver_timer(aid(1), 55);
        assert_eq!(
            d.next_event(aid(1)),
            Some(BehaviorEvent::Message(20, b"m".to_vec()))
        );
        assert_eq!(d.next_event(aid(1)), Some(BehaviorEvent::Timer(55)));
    }

    #[test]
    fn clear_drops_queued_events() {
        let mut d = EventDispatcher::new();
        d.subscribe(aid(1), 3);
        d.route_message(3, b"x");
        d.clear(aid(1));
        assert_eq!(d.pending(aid(1)), 0);
        assert_eq!(d.next_event(aid(1)), None);
    }
}
