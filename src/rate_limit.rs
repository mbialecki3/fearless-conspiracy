use crate::game::PlayerId;
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

const COMMAND_WINDOW: Duration = Duration::from_secs(1);
const CHAT_WINDOW: Duration = Duration::from_secs(5);
const VIOLATION_WINDOW: Duration = Duration::from_secs(10);
const COMMAND_LIMIT: usize = 20;
const CHAT_LIMIT: usize = 5;
const DISCONNECT_VIOLATIONS: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateDecision {
    Allowed,
    Rejected,
    Disconnect,
}

#[derive(Default)]
struct PlayerRate {
    commands: VecDeque<Duration>,
    chats: VecDeque<Duration>,
    violations: VecDeque<Duration>,
}

#[derive(Default)]
pub struct RateLimiter {
    players: HashMap<PlayerId, PlayerRate>,
}

impl RateLimiter {
    pub fn check(&mut self, player: PlayerId, is_chat: bool, now: Duration) -> RateDecision {
        let rate = self.players.entry(player).or_default();
        prune(&mut rate.commands, now, COMMAND_WINDOW);
        prune(&mut rate.chats, now, CHAT_WINDOW);
        prune(&mut rate.violations, now, VIOLATION_WINDOW);
        let limited =
            rate.commands.len() >= COMMAND_LIMIT || (is_chat && rate.chats.len() >= CHAT_LIMIT);
        if limited {
            rate.violations.push_back(now);
            if rate.violations.len() >= DISCONNECT_VIOLATIONS {
                RateDecision::Disconnect
            } else {
                RateDecision::Rejected
            }
        } else {
            rate.commands.push_back(now);
            if is_chat {
                rate.chats.push_back(now);
            }
            RateDecision::Allowed
        }
    }

    pub fn remove(&mut self, player: PlayerId) {
        self.players.remove(&player);
    }
}

fn prune(queue: &mut VecDeque<Duration>, now: Duration, window: Duration) {
    while queue
        .front()
        .is_some_and(|timestamp| now.saturating_sub(*timestamp) >= window)
    {
        queue.pop_front();
    }
}

#[derive(Default)]
pub struct RequestTracker {
    latest: HashMap<PlayerId, u64>,
}

impl RequestTracker {
    pub fn accept(&mut self, player: PlayerId, request_id: u64) -> bool {
        match self.latest.get_mut(&player) {
            Some(latest) if request_id <= *latest => false,
            Some(latest) => {
                *latest = request_id;
                true
            }
            None => {
                self.latest.insert(player, request_id);
                true
            }
        }
    }

    pub fn remove(&mut self, player: PlayerId) {
        self.latest.remove(&player);
    }
}

pub fn is_chat_command(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.starts_with('/')
        || ["/say ", "/whisper ", "/mafia ", "/jailchat ", "/dead "]
            .iter()
            .any(|prefix| trimmed.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixth_chat_in_five_seconds_is_rejected() {
        let mut limiter = RateLimiter::default();
        for index in 0..5 {
            assert_eq!(
                limiter.check(PlayerId(1), true, Duration::from_millis(index * 10)),
                RateDecision::Allowed
            );
        }
        assert_eq!(
            limiter.check(PlayerId(1), true, Duration::from_millis(100)),
            RateDecision::Rejected
        );
        assert_eq!(
            limiter.check(PlayerId(1), true, Duration::from_secs(5)),
            RateDecision::Allowed
        );
    }

    #[test]
    fn repeated_limit_violations_request_disconnect() {
        let mut limiter = RateLimiter::default();
        for index in 0..20 {
            assert_eq!(
                limiter.check(PlayerId(1), false, Duration::from_millis(index)),
                RateDecision::Allowed
            );
        }
        for index in 0..9 {
            assert_eq!(
                limiter.check(PlayerId(1), false, Duration::from_millis(100 + index)),
                RateDecision::Rejected
            );
        }
        assert_eq!(
            limiter.check(PlayerId(1), false, Duration::from_millis(200)),
            RateDecision::Disconnect
        );
    }

    #[test]
    fn duplicate_and_old_request_ids_are_rejected() {
        let mut tracker = RequestTracker::default();
        assert!(tracker.accept(PlayerId(1), 10));
        assert!(!tracker.accept(PlayerId(1), 10));
        assert!(!tracker.accept(PlayerId(1), 9));
        assert!(tracker.accept(PlayerId(1), 11));
        assert!(tracker.accept(PlayerId(2), 1));
    }

    #[test]
    fn chat_detection_covers_bare_and_explicit_channels() {
        assert!(is_chat_command("hello"));
        assert!(is_chat_command("/whisper 2 hello"));
        assert!(!is_chat_command("/vote 2"));
    }
}
