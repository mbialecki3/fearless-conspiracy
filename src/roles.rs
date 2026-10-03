use crate::game::PlayerId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Faction {
    Town,
    Mafia,
    Neutral,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Sheriff,
    Doctor,
    Lookout,
    TavernKeeper,
    Jailor,
    Godfather,
    Mafioso,
    Framer,
    Jester,
}

impl Role {
    pub const ALL: [Self; 9] = [
        Self::Sheriff,
        Self::Doctor,
        Self::Lookout,
        Self::TavernKeeper,
        Self::Jailor,
        Self::Godfather,
        Self::Mafioso,
        Self::Framer,
        Self::Jester,
    ];

    pub const fn faction(self) -> Faction {
        match self {
            Self::Sheriff | Self::Doctor | Self::Lookout | Self::TavernKeeper | Self::Jailor => {
                Faction::Town
            }
            Self::Godfather | Self::Mafioso | Self::Framer => Faction::Mafia,
            Self::Jester => Faction::Neutral,
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Sheriff => "Sheriff",
            Self::Doctor => "Doctor",
            Self::Lookout => "Lookout",
            Self::TavernKeeper => "Tavern Keeper (Escort)",
            Self::Jailor => "Jailor",
            Self::Godfather => "Godfather",
            Self::Mafioso => "Mafioso",
            Self::Framer => "Framer",
            Self::Jester => "Jester",
        }
    }

    pub const fn appears_suspicious(self) -> bool {
        matches!(self, Self::Mafioso | Self::Framer)
    }

    pub const fn has_basic_defense(self) -> bool {
        matches!(self, Self::Godfather)
    }

    pub const fn is_mafia_killing(self) -> bool {
        matches!(self, Self::Godfather | Self::Mafioso)
    }

    pub const fn night_command(self) -> Option<&'static str> {
        match self {
            Self::Sheriff => Some("investigate"),
            Self::Doctor => Some("heal"),
            Self::Lookout => Some("watch"),
            Self::TavernKeeper => Some("block"),
            Self::Jailor => Some("execute"),
            Self::Godfather | Self::Mafioso => Some("kill"),
            Self::Framer => Some("frame"),
            Self::Jester => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Demo5,
    Standard7,
    Standard8,
    Full9,
}

impl Preset {
    pub fn roles(self) -> &'static [Role] {
        use Role::*;
        match self {
            Self::Demo5 => &[Sheriff, Doctor, Lookout, Mafioso, Jester],
            Self::Standard7 => &[Sheriff, Doctor, Lookout, Jailor, Godfather, Mafioso, Jester],
            Self::Standard8 => &[
                Sheriff,
                Doctor,
                Lookout,
                Jailor,
                Godfather,
                Mafioso,
                Jester,
                TavernKeeper,
            ],
            Self::Full9 => &Role::ALL,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Demo5 => "Demo 5",
            Self::Standard7 => "Standard 7",
            Self::Standard8 => "Standard 8",
            Self::Full9 => "Full 9",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NightIntent {
    Investigate(PlayerId),
    Heal(PlayerId),
    Watch(PlayerId),
    Block(PlayerId),
    Kill(PlayerId),
    Frame(PlayerId),
    Execute,
    Spare,
    Haunt(PlayerId),
}

impl NightIntent {
    pub const fn target(self) -> Option<PlayerId> {
        match self {
            Self::Investigate(target)
            | Self::Heal(target)
            | Self::Watch(target)
            | Self::Block(target)
            | Self::Kill(target)
            | Self::Frame(target)
            | Self::Haunt(target) => Some(target),
            Self::Execute | Self::Spare => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AttackStrength {
    Basic,
    Unstoppable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DefenseStrength {
    None,
    Basic,
    Powerful,
}

impl AttackStrength {
    pub const fn defeats(self, defense: DefenseStrength) -> bool {
        match self {
            Self::Basic => matches!(defense, DefenseStrength::None),
            Self::Unstoppable => true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Visit {
    pub actor: PlayerId,
    pub target: PlayerId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attack {
    pub actor: PlayerId,
    pub target: PlayerId,
    pub strength: AttackStrength,
    pub cause: DeathCause,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeathCause {
    Mafia,
    Jailor,
    JesterHaunt,
    PublicExecution,
    Forfeit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeathRecord {
    pub player: PlayerId,
    pub causes: BTreeSet<DeathCause>,
    pub role: Role,
    pub will: String,
    pub death_note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HauntState {
    pub jester: PlayerId,
    pub eligible: BTreeSet<PlayerId>,
}

#[derive(Clone, Debug, Default)]
pub struct NightOutcome {
    pub visits: Vec<Visit>,
    pub attacks: Vec<Attack>,
    pub deaths: Vec<DeathRecord>,
    pub actual_mafia_attacker: Option<PlayerId>,
    pub blocked: BTreeSet<PlayerId>,
    pub events: Vec<crate::game::GameEvent>,
}

pub fn resolve_night(game: &mut crate::game::Game) -> Result<NightOutcome, crate::game::GameError> {
    use crate::game::{GameEvent, MatchOutcome, PhaseKind, Recipients};
    use rand::seq::{IteratorRandom, SliceRandom};
    use std::collections::{BTreeMap, BTreeSet};

    let intentions = game.intents.clone();
    let mut outcome = NightOutcome::default();

    for (&actor, &intent) in &intentions {
        let player = game.player(actor)?;
        if !player.alive || matches!(intent, NightIntent::Haunt(_)) {
            continue;
        }
        if let Some(target) = intent.target() {
            let is_non_visit = matches!(intent, NightIntent::Heal(id) if id == actor);
            if !is_non_visit {
                outcome.visits.push(Visit { actor, target });
            }
        }
    }

    let godfather = living_with_role(game, Role::Godfather);
    let mafioso = living_with_role(game, Role::Mafioso);
    let godfather_target = godfather.and_then(|actor| kill_target(&intentions, actor));
    let mafioso_target = mafioso.and_then(|actor| kill_target(&intentions, actor));
    let mafia_attack = match (godfather, godfather_target, mafioso, mafioso_target) {
        (_, Some(target), Some(attacker), _) => Some((attacker, target)),
        (_, None, Some(attacker), Some(target)) => Some((attacker, target)),
        (Some(attacker), Some(target), None, _) => Some((attacker, target)),
        _ => None,
    };
    if let Some((attacker, target)) = mafia_attack {
        outcome.actual_mafia_attacker = Some(attacker);
        outcome.visits.retain(|visit| {
            !matches!(
                game.player(visit.actor).and_then(|p| p.role()),
                Ok(Role::Godfather | Role::Mafioso)
            )
        });
        outcome.visits.push(Visit {
            actor: attacker,
            target,
        });
        outcome.attacks.push(Attack {
            actor: attacker,
            target,
            strength: AttackStrength::Basic,
            cause: DeathCause::Mafia,
        });
    }

    let mut protected = BTreeMap::<PlayerId, PlayerId>::new();
    for (&actor, &intent) in &intentions {
        if let NightIntent::Heal(target) = intent
            && game.player(actor)?.alive
            && game.player(actor)?.role()? == Role::Doctor
        {
            protected.insert(target, actor);
            if target == actor {
                game.player_mut(actor)?.self_heals_remaining -= 1;
            }
        }
    }

    if let Some(haunt) = game.pending_haunt.clone() {
        let selected = intentions
            .get(&haunt.jester)
            .and_then(|intent| match intent {
                NightIntent::Haunt(target)
                    if haunt.eligible.contains(target)
                        && game.player(*target).is_ok_and(|p| p.alive) =>
                {
                    Some(*target)
                }
                _ => None,
            });
        let living_eligible: Vec<_> = haunt
            .eligible
            .iter()
            .copied()
            .filter(|target| game.player(*target).is_ok_and(|player| player.alive))
            .collect();
        let target = selected.or_else(|| living_eligible.into_iter().choose(&mut game.rng));
        if let Some(target) = target {
            outcome.attacks.push(Attack {
                actor: haunt.jester,
                target,
                strength: AttackStrength::Unstoppable,
                cause: DeathCause::JesterHaunt,
            });
        }
        game.pending_haunt = None;
    }

    for (&actor, &intent) in &intentions {
        match intent {
            NightIntent::Investigate(target)
                if game.player(actor)?.alive && game.player(actor)?.role()? == Role::Sheriff =>
            {
                let suspicious = game.player(target)?.role()?.appears_suspicious();
                outcome.events.push(GameEvent {
                    recipients: Recipients::only(actor),
                    channel: "private_system",
                    text: if suspicious {
                        format!(
                            "Night {}: {target} appears suspicious. Framing is possible.",
                            game.phase.day
                        )
                    } else {
                        format!(
                            "Night {}: no suspicious activity found for {target}. This does not prove Town alignment.",
                            game.phase.day
                        )
                    },
                });
            }
            NightIntent::Watch(target)
                if game.player(actor)?.alive && game.player(actor)?.role()? == Role::Lookout =>
            {
                let mut visitors: Vec<_> = outcome
                    .visits
                    .iter()
                    .filter(|visit| visit.target == target && visit.actor != actor)
                    .map(|visit| visit.actor)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let additional = visitors.len() > 3;
                if additional {
                    visitors.shuffle(&mut game.rng);
                    visitors.truncate(3);
                    visitors.sort();
                }
                let report = if visitors.is_empty() {
                    "no visitors".into()
                } else {
                    visitors
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                outcome.events.push(GameEvent {
                    recipients: Recipients::only(actor),
                    channel: "private_system",
                    text: format!(
                        "Night {}: visitors to {target}: {report}{}.",
                        game.phase.day,
                        if additional {
                            "; additional visitors were seen"
                        } else {
                            ""
                        }
                    ),
                });
            }
            _ => {}
        }
    }

    let mut lethal = BTreeMap::<PlayerId, BTreeSet<DeathCause>>::new();
    for attack in &outcome.attacks {
        let role_defense = if game.player(attack.target)?.role()?.has_basic_defense() {
            DefenseStrength::Basic
        } else {
            DefenseStrength::None
        };
        let defense = if protected.contains_key(&attack.target) {
            DefenseStrength::Powerful
        } else {
            role_defense
        };
        if attack.strength.defeats(defense) {
            lethal
                .entry(attack.target)
                .or_default()
                .insert(attack.cause);
        }
    }

    for (&target, &doctor) in &protected {
        if outcome.attacks.iter().any(|attack| attack.target == target) {
            outcome.events.push(GameEvent {
                recipients: Recipients::only(doctor),
                channel: "private_system",
                text: format!("Your protected target {target} was attacked."),
            });
            if outcome.attacks.iter().any(|attack| {
                attack.target == target && !attack.strength.defeats(DefenseStrength::Powerful)
            }) {
                outcome.events.push(GameEvent {
                    recipients: Recipients::only(target),
                    channel: "private_system",
                    text: "You were attacked and protected; the Doctor's identity remains hidden."
                        .into(),
                });
            }
        }
    }
    if let Some((attacker, target)) = mafia_attack
        && !lethal
            .get(&target)
            .is_some_and(|causes| causes.contains(&DeathCause::Mafia))
    {
        outcome.events.push(GameEvent {
            recipients: Recipients::only(attacker),
            channel: "private_system",
            text: format!("The attack on {target} failed."),
        });
    }

    for (target, causes) in lethal {
        let player = game.player(target)?;
        let death_note = causes.contains(&DeathCause::Mafia).then(|| {
            mafia_attack
                .and_then(|(attacker, _)| game.player(attacker).ok())
                .map(|attacker| attacker.death_note.clone())
                .unwrap_or_default()
        });
        outcome.deaths.push(DeathRecord {
            player: target,
            causes,
            role: player.role()?,
            will: player.will.clone(),
            death_note,
        });
    }
    outcome.deaths.sort_by_key(|death| death.player);
    for death in &outcome.deaths {
        game.player_mut(death.player)?.alive = false;
    }

    if outcome.deaths.is_empty() {
        outcome.events.push(GameEvent {
            recipients: Recipients::All,
            channel: "public_system",
            text: "Nobody died during the night.".into(),
        });
    } else {
        for death in &outcome.deaths {
            let name = &game.player(death.player)?.name;
            let note = death
                .death_note
                .as_deref()
                .filter(|note| !note.is_empty())
                .map(|note| format!(" Death note: {note}"))
                .unwrap_or_default();
            outcome.events.push(GameEvent {
                recipients: Recipients::All,
                channel: "public_system",
                text: format!(
                    "{} {} died. Causes: {:?}. Role: {}. Will: {}{}",
                    death.player,
                    name,
                    death.causes,
                    death.role.display_name(),
                    if death.will.is_empty() {
                        "(empty)"
                    } else {
                        &death.will
                    },
                    note
                ),
            });
        }
    }

    game.intents.clear();
    game.completed_nights = game.completed_nights.saturating_add(1);
    if game.phase.day >= 2 {
        if outcome.deaths.is_empty() {
            game.deathless_cycles = game.deathless_cycles.saturating_add(1);
        } else {
            game.deathless_cycles = 0;
        }
    }
    let dawn_day = game.phase.day.saturating_add(1);
    game.move_to(PhaseKind::Dawn, dawn_day);
    let faction_outcome = game.evaluate_victory();
    if faction_outcome == MatchOutcome::InProgress
        && (game.deathless_cycles >= 3 || game.completed_nights >= 20)
    {
        game.outcome = MatchOutcome::Draw;
        game.move_to(PhaseKind::Ended, dawn_day);
        outcome.events.push(GameEvent {
            recipients: Recipients::All,
            channel: "public_system",
            text: "The faction contest ended in a draw.".into(),
        });
    } else if faction_outcome != MatchOutcome::InProgress {
        outcome.events.push(GameEvent {
            recipients: Recipients::All,
            channel: "public_system",
            text: format!("Match result: {faction_outcome:?}."),
        });
    }

    Ok(outcome)
}

fn living_with_role(game: &crate::game::Game, role: Role) -> Option<PlayerId> {
    game.players
        .values()
        .find(|player| player.alive && player.current_role == Some(role))
        .map(|player| player.id)
}

fn kill_target(
    intentions: &std::collections::BTreeMap<PlayerId, NightIntent>,
    actor: PlayerId,
) -> Option<PlayerId> {
    match intentions.get(&actor) {
        Some(NightIntent::Kill(target)) => Some(*target),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Game, GameConfig, MatchOutcome, PhaseKind, PlayerId, Recipients};

    fn game_at_night(roles: Vec<Role>, seed: u64) -> Game {
        let mut game = Game::with_seed(GameConfig::scenario(roles), seed).unwrap();
        let count = game.config.expected_players();
        for number in 1..=count {
            let id = game
                .join(4_000 + number as u32, &format!("P{number}"))
                .unwrap();
            game.toggle_ready(id).unwrap();
        }
        game.start().unwrap();
        game.advance_phase(game.phase.id).unwrap();
        game.advance_phase(game.phase.id).unwrap();
        assert_eq!(game.phase.kind, PhaseKind::Night);
        game
    }

    #[test]
    fn protected_attack_fails_but_lookout_evidence_survives() {
        let mut game = game_at_night(Preset::Demo5.roles().to_vec(), 11);
        let phase = game.phase.id;
        game.select_night_intent(PlayerId(1), NightIntent::Investigate(PlayerId(4)), phase)
            .unwrap();
        game.select_night_intent(PlayerId(2), NightIntent::Heal(PlayerId(2)), phase)
            .unwrap();
        game.select_night_intent(PlayerId(3), NightIntent::Watch(PlayerId(3)), phase)
            .unwrap_err();
        game.select_night_intent(PlayerId(3), NightIntent::Watch(PlayerId(2)), phase)
            .unwrap();
        game.select_night_intent(PlayerId(4), NightIntent::Kill(PlayerId(2)), phase)
            .unwrap();

        let outcome = game.resolve_night().unwrap();
        assert!(outcome.deaths.is_empty());
        assert_eq!(outcome.actual_mafia_attacker, Some(PlayerId(4)));
        assert!(outcome.visits.contains(&Visit {
            actor: PlayerId(4),
            target: PlayerId(2)
        }));
        let lookout = outcome
            .events
            .iter()
            .find(|event| event.recipients == Recipients::only(PlayerId(3)))
            .unwrap();
        assert!(lookout.text.contains("#4"));
        assert!(outcome.events.iter().any(|event| {
            event.recipients == Recipients::only(PlayerId(1))
                && event.text.contains("appears suspicious")
        }));
        assert!(game.player(PlayerId(2)).unwrap().alive);
    }

    #[test]
    fn unprotected_mafia_attack_freezes_will_and_publishes_death_note() {
        let mut game = game_at_night(Preset::Demo5.roles().to_vec(), 12);
        game.set_will(PlayerId(1), "Sheriff notes").unwrap();
        game.set_death_note(PlayerId(4), "The shadows came")
            .unwrap();
        game.select_night_intent(PlayerId(4), NightIntent::Kill(PlayerId(1)), game.phase.id)
            .unwrap();
        let outcome = game.resolve_night().unwrap();
        assert_eq!(outcome.deaths.len(), 1);
        assert_eq!(outcome.deaths[0].player, PlayerId(1));
        assert_eq!(outcome.deaths[0].will, "Sheriff notes");
        assert_eq!(
            outcome.deaths[0].death_note.as_deref(),
            Some("The shadows came")
        );
        assert!(!game.player(PlayerId(1)).unwrap().alive);
    }

    #[test]
    fn self_heal_is_spent_when_it_activates_without_an_attack() {
        let mut game = game_at_night(Preset::Demo5.roles().to_vec(), 13);
        game.select_night_intent(PlayerId(2), NightIntent::Heal(PlayerId(2)), game.phase.id)
            .unwrap();
        game.resolve_night().unwrap();
        assert_eq!(game.player(PlayerId(2)).unwrap().self_heals_remaining, 0);
    }

    #[test]
    fn haunt_bypasses_doctor_protection_and_preserves_jester_win() {
        let mut game = game_at_night(Preset::Demo5.roles().to_vec(), 14);
        game.player_mut(PlayerId(5)).unwrap().alive = false;
        game.player_mut(PlayerId(5)).unwrap().jester_won = true;
        game.pending_haunt = Some(HauntState {
            jester: PlayerId(5),
            eligible: BTreeSet::from([PlayerId(1)]),
        });
        let phase = game.phase.id;
        game.select_night_intent(PlayerId(2), NightIntent::Heal(PlayerId(1)), phase)
            .unwrap();
        game.select_night_intent(PlayerId(5), NightIntent::Haunt(PlayerId(1)), phase)
            .unwrap();
        let outcome = game.resolve_night().unwrap();
        assert!(outcome.deaths[0].causes.contains(&DeathCause::JesterHaunt));
        assert!(game.player(PlayerId(5)).unwrap().jester_won);
    }

    #[test]
    fn frozen_intentions_resolve_identically_regardless_of_submission_order() {
        fn run(order: &[PlayerId]) -> (Vec<Visit>, Vec<DeathRecord>, MatchOutcome) {
            let mut game = game_at_night(Preset::Demo5.roles().to_vec(), 99);
            for actor in order {
                let intent = match actor.0 {
                    1 => NightIntent::Investigate(PlayerId(4)),
                    2 => NightIntent::Heal(PlayerId(1)),
                    3 => NightIntent::Watch(PlayerId(1)),
                    4 => NightIntent::Kill(PlayerId(1)),
                    _ => unreachable!(),
                };
                game.select_night_intent(*actor, intent, game.phase.id)
                    .unwrap();
            }
            let result = game.resolve_night().unwrap();
            (result.visits, result.deaths, game.outcome)
        }
        assert_eq!(
            run(&[PlayerId(1), PlayerId(2), PlayerId(3), PlayerId(4)]),
            run(&[PlayerId(4), PlayerId(2), PlayerId(1), PlayerId(3)])
        );
    }
}
