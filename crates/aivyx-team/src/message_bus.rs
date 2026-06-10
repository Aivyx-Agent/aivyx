//! The team `MessageBus` — inter-specialist (and lead↔specialist) dialogue
//! (J.3.1).
//!
//! A bounded **tokio broadcast** channel: every subscriber sees every
//! message (fan-out), and the bound means a slow reader **drops old
//! messages** with a `Lagged` signal rather than blocking the team. Reads
//! are non-blocking `try_recv` drains, so the bus core is fully
//! synchronous — the `send`/`read_message` tools (J.3.2) wrap it.

use std::sync::Arc;

use tokio::sync::broadcast;

/// Who a message is addressed to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recipient {
    /// Every team member.
    Broadcast,
    /// One named member.
    Agent(String),
}

/// A message on the team bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamMessage {
    pub from: String,
    pub to: Recipient,
    pub content: String,
}

impl TeamMessage {
    /// A message to the whole team.
    pub fn broadcast(from: impl Into<String>, content: impl Into<String>) -> Self {
        TeamMessage {
            from: from.into(),
            to: Recipient::Broadcast,
            content: content.into(),
        }
    }

    /// A direct message to one member.
    pub fn direct(
        from: impl Into<String>,
        to: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        TeamMessage {
            from: from.into(),
            to: Recipient::Agent(to.into()),
            content: content.into(),
        }
    }

    /// Whether `agent` should receive this message: it's a broadcast or
    /// addressed to `agent`, and `agent` is not the sender (you don't read
    /// back your own messages).
    pub fn visible_to(&self, agent: &str) -> bool {
        if self.from == agent {
            return false;
        }
        match &self.to {
            Recipient::Broadcast => true,
            Recipient::Agent(name) => name == agent,
        }
    }
}

/// The shared team message bus.
pub struct MessageBus {
    tx: broadcast::Sender<TeamMessage>,
}

impl MessageBus {
    /// A bus with the given per-subscriber buffer (from
    /// `DialogueConfig::message_bus_capacity`; floored to 1).
    pub fn new(capacity: usize) -> Arc<Self> {
        let (tx, _rx) = broadcast::channel(capacity.max(1));
        Arc::new(MessageBus { tx })
    }

    /// Publish a message. Returns the number of live subscribers it reached
    /// (0 when no one is listening — not an error).
    pub fn publish(&self, msg: TeamMessage) -> usize {
        self.tx.send(msg).unwrap_or(0)
    }

    /// Subscribe as `agent`. Only messages sent **after** this call are
    /// delivered (broadcast semantics).
    pub fn subscribe(&self, agent: impl Into<String>) -> Subscription {
        Subscription {
            agent: agent.into(),
            rx: self.tx.subscribe(),
        }
    }
}

/// One member's read-side view of the bus.
pub struct Subscription {
    agent: String,
    rx: broadcast::Receiver<TeamMessage>,
}

/// The result of draining a [`Subscription`].
#[derive(Debug, Default, PartialEq)]
pub struct Drained {
    /// Messages visible to this member, oldest first.
    pub messages: Vec<TeamMessage>,
    /// How many messages were dropped because this member fell behind the
    /// buffer (a backpressure signal — read more often, or raise capacity).
    pub lagged: u64,
}

impl Subscription {
    /// Drain all currently-pending messages visible to this member. Never
    /// blocks; messages addressed elsewhere are skipped; a lag is reported.
    pub fn drain(&mut self) -> Drained {
        use broadcast::error::TryRecvError;
        let mut out = Drained::default();
        loop {
            match self.rx.try_recv() {
                Ok(msg) => {
                    if msg.visible_to(&self.agent) {
                        out.messages.push(msg);
                    }
                }
                Err(TryRecvError::Lagged(n)) => out.lagged += n,
                Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_rules() {
        let bcast = TeamMessage::broadcast("lead", "stand-up");
        assert!(bcast.visible_to("inventory"), "broadcast reaches others");
        assert!(!bcast.visible_to("lead"), "sender doesn't read its own");

        let direct = TeamMessage::direct("lead", "haccp", "log the fridge");
        assert!(direct.visible_to("haccp"), "addressee sees it");
        assert!(!direct.visible_to("inventory"), "non-addressee does not");
        assert!(!direct.visible_to("lead"), "sender doesn't read its own");
    }

    #[test]
    fn broadcast_fans_out_to_all_subscribers() {
        let bus = MessageBus::new(16);
        let mut a = bus.subscribe("a");
        let mut b = bus.subscribe("b");
        assert_eq!(bus.publish(TeamMessage::broadcast("lead", "hello")), 2);
        assert_eq!(a.drain().messages.len(), 1);
        assert_eq!(b.drain().messages.len(), 1);
        // Drained — nothing left.
        assert!(a.drain().messages.is_empty());
    }

    #[test]
    fn direct_messages_reach_only_the_addressee() {
        let bus = MessageBus::new(16);
        let mut haccp = bus.subscribe("haccp");
        let mut inventory = bus.subscribe("inventory");
        bus.publish(TeamMessage::direct("lead", "haccp", "log it"));
        assert_eq!(haccp.drain().messages.len(), 1);
        assert!(inventory.drain().messages.is_empty(), "not addressed to it");
    }

    #[test]
    fn a_sender_does_not_receive_its_own_broadcast() {
        let bus = MessageBus::new(16);
        let mut lead = bus.subscribe("lead");
        bus.publish(TeamMessage::broadcast("lead", "to everyone but me"));
        assert!(lead.drain().messages.is_empty());
    }

    #[test]
    fn subscribing_after_a_publish_misses_it() {
        let bus = MessageBus::new(16);
        bus.publish(TeamMessage::broadcast("lead", "early"));
        let mut late = bus.subscribe("late");
        assert!(late.drain().messages.is_empty(), "only post-subscribe messages");
    }

    #[test]
    fn falling_behind_capacity_reports_a_lag_and_keeps_recent() {
        let bus = MessageBus::new(2); // tiny buffer
        let mut slow = bus.subscribe("slow");
        for i in 0..5 {
            bus.publish(TeamMessage::broadcast("lead", format!("msg {i}")));
        }
        let drained = slow.drain();
        assert!(drained.lagged > 0, "the slow reader dropped old messages");
        assert!(
            !drained.messages.is_empty(),
            "but still gets the most recent within the buffer"
        );
        // The kept messages are the most recent.
        let last = &drained.messages.last().unwrap().content;
        assert_eq!(last, "msg 4");
    }

    #[test]
    fn publish_with_no_subscribers_is_not_an_error() {
        let bus = MessageBus::new(4);
        assert_eq!(bus.publish(TeamMessage::broadcast("lead", "anyone?")), 0);
    }
}
