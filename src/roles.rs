use crate::game::PlayerId;
use serde::{Deserialize, Serialize};

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
