//! The launcher-backed nav entries — NEWS, ITEM STORE, WORKSHOP — and the
//! honest state each of them is in.
//!
//! **These three exist because the platform has them** (operator, 2026-08-09:
//! *"we have NEWS thats part of the community on the scry-works launcher …
//! we do have an item store and workshop. that again is part of the
//! scry-works setup"*). The first cut of the shell left all three off the nav
//! on the grounds that nothing was behind them, which was wrong about the
//! product rather than wrong about the rule: `settings.rs`'s policy is that a
//! category with nothing behind it *says so*, and the way to honour it here
//! is to draw the entry and state its condition — not to hide a section of
//! the product because this client has not reached it yet.
//!
//! ## Why a link and not a screen, and this is the load-bearing decision
//!
//! Every one of the three resolves to a URL the **launcher** opens, in the
//! launcher's own window. That is not laziness about drawing a store; it is
//! the rule the vendored SDK states on `Overlay::overlay`:
//!
//! > ⚠ **Display freely; never approve.** A game controls its own pixels, so
//! > it can draw a convincing fake of any dialog.
//!
//! A game that draws its own checkout is a game teaching its players that a
//! checkout drawn by a game is normal — and the next one they see will be a
//! forgery. So the money door is the launcher's, always, and the same
//! reasoning covers workshop submissions (they are signed) and leaves news as
//! the one section that *could* be drawn in-game later without breaking
//! anything. `crate::manifest` is where the URLs come from.
//!
//! ## The four states, and why none of them may be collapsed
//!
//! [`Status`] is deliberately four-valued rather than `Option<&str>`. The
//! same argument the SDK makes about `Overlay::profile` — *"three states, and
//! a game that collapses them tells a lie"* — applies exactly: "no launcher
//! is running", "the launcher gave us no manifest", "the manifest names no
//! link for this section" and "here is the link" are four different problems
//! with four different fixes, and a screen that draws one greyed row for all
//! of them tells the player nothing they can act on.

use crate::manifest::Manifest;

/// One launcher-backed nav entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    News,
    Store,
    Workshop,
}

impl Section {
    /// In the reference's own order, which is also the order of how often a
    /// player wants them.
    pub const ALL: [Section; 3] = [Section::News, Section::Store, Section::Workshop];

    /// The nav label. Overridable by the manifest ([`Section::label`]) so the
    /// platform can rename a section without a client release.
    pub fn default_label(self) -> &'static str {
        match self {
            Section::News => "NEWS",
            Section::Store => "ITEM STORE",
            Section::Workshop => "WORKSHOP",
        }
    }

    /// What the pane says this section *is*. Written so a player who has
    /// never opened the launcher still knows what they are being offered.
    pub fn blurb(self) -> &'static str {
        match self {
            Section::News => {
                "Patch notes, wipe announcements and what the community is \
                 posting - published on scry-works and shared by every game \
                 on it."
            }
            Section::Store => {
                "Skins and cosmetics. Never a stat: nothing sold here changes \
                 damage, gather rate, or anything else another player has to \
                 live with."
            }
            Section::Workshop => {
                "Community-made items. Make one, put it up, and earn when \
                 players use it."
            }
        }
    }

    /// The one sentence that says why the button hands off instead of drawing
    /// the thing. Shown on every section, because the reason is the same one
    /// and a player should be able to learn it from any of them.
    pub fn handoff_reason(self) -> &'static str {
        match self {
            // The only one whose reason is convenience rather than safety —
            // said plainly rather than dressed up as a rule.
            Section::News => "opens in the launcher, where the rest of the community is",
            Section::Store => {
                "opens in the launcher. A game must never take a payment in its own \
                 window - it draws its own pixels, so it could draw a convincing fake \
                 of one, and the launcher is where a real checkout lives"
            }
            Section::Workshop => {
                "opens in the launcher, which holds the key a submission is signed with"
            }
        }
    }

    /// This section's entry in a fetched manifest.
    fn of(self, m: &Manifest) -> &crate::manifest::Section {
        match self {
            Section::News => &m.news,
            Section::Store => &m.store,
            Section::Workshop => &m.workshop,
        }
    }

    /// The label to draw: the manifest's, if it named one that is usable.
    ///
    /// A blank or whitespace override is ignored rather than drawn — an empty
    /// nav entry is a hole a player cannot click with any confidence, and it
    /// would be caused by a document nobody in this repo controls.
    pub fn label(self, hub: &Hub) -> String {
        hub.manifest
            .as_ref()
            .and_then(|m| self.of(m).label.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(self.default_label())
            .to_string()
    }

    /// What this section can do right now.
    pub fn status(self, hub: &Hub) -> Status {
        if !hub.launcher {
            return Status::NoLauncher;
        }
        match &hub.manifest {
            None => match &hub.why {
                Some(why) => Status::NoManifest(why.clone()),
                None => Status::Fetching,
            },
            Some(m) => match self.of(m).link() {
                Some(url) => Status::Ready(url.to_string()),
                None => Status::NoLink,
            },
        }
    }
}

/// What a section can do, and — when it cannot — what would fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Nothing to ask. The majority case and never drawn as a fault: the
    /// game is playable without a launcher and always has been.
    NoLauncher,
    /// The launcher answered and the manifest is still in flight.
    Fetching,
    /// We asked and could not get one. Carries the reason verbatim.
    NoManifest(String),
    /// The manifest is here and names nothing for this section.
    NoLink,
    /// Openable.
    Ready(String),
}

impl Status {
    /// The URL, if this is the one state that has one.
    pub fn url(&self) -> Option<&str> {
        match self {
            Status::Ready(u) => Some(u),
            _ => None,
        }
    }

    /// The line under the section's name — always something, and it always
    /// names what would change it. The rule the shard list has obeyed since
    /// it existed, applied to a different document.
    pub fn line(&self) -> String {
        match self {
            Status::NoLauncher => "the scry launcher is not running - start the game from it \
                                   to reach this"
                .to_string(),
            Status::Fetching => "asking the launcher where this lives...".to_string(),
            Status::NoManifest(why) => why.clone(),
            Status::NoLink => {
                "the launcher's manifest names no link for this yet - nothing is wrong, \
                 it is not published"
                    .to_string()
            }
            Status::Ready(u) => u.clone(),
        }
    }
}

/// Everything the three sections are drawn from.
///
/// One resource for all three because they answer to one document: fetching
/// per section would be three GETs of the same file and three ways for them
/// to disagree about what it said.
#[derive(Debug, Clone, Default)]
pub struct Hub {
    /// Is a launcher connected at all?
    pub launcher: bool,
    /// The manifest, once fetched.
    pub manifest: Option<Manifest>,
    /// Why there is no manifest, if the attempt has finished and failed.
    /// `None` with `manifest: None` means the attempt is still out — which is
    /// what keeps [`Status::Fetching`] distinct from [`Status::NoManifest`].
    pub why: Option<String>,
}

impl Hub {
    /// Is there anything left to wait for? Drives whether the screen redraws
    /// itself when the fetch lands.
    pub fn settled(&self) -> bool {
        !self.launcher || self.manifest.is_some() || self.why.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(json: &str) -> Hub {
        Hub {
            launcher: true,
            manifest: Some(crate::manifest::parse(json.as_bytes()).unwrap()),
            why: None,
        }
    }

    #[test]
    fn the_four_states_are_four_different_sentences() {
        // The SDK's own rule about `profile`, applied here: collapsing these
        // tells the player nothing they can act on. Each names its own fix.
        let no_launcher = Hub::default();
        let fetching = Hub {
            launcher: true,
            ..Hub::default()
        };
        let failed = Hub {
            launcher: true,
            why: Some("title manifest: not readable JSON".into()),
            ..Hub::default()
        };
        let empty = with("{}");
        let ready = with(r#"{"news":{"url":"https://scry.works/gates/news"}}"#);

        let s = Section::News;
        let lines: Vec<String> = [&no_launcher, &fetching, &failed, &empty, &ready]
            .iter()
            .map(|h| s.status(h).line())
            .collect();
        for (i, a) in lines.iter().enumerate() {
            assert!(!a.is_empty(), "a state with nothing to say is not a state");
            for b in lines.iter().skip(i + 1) {
                assert_ne!(a, b, "two states say the same thing");
            }
        }

        // Only one of them is openable, and it is the only one carrying a url.
        assert_eq!(
            s.status(&ready).url(),
            Some("https://scry.works/gates/news")
        );
        for h in [&no_launcher, &fetching, &failed, &empty] {
            assert_eq!(s.status(h).url(), None);
        }
    }

    #[test]
    fn no_launcher_beats_every_other_answer() {
        // A stale manifest with no launcher to open a link with must not draw
        // a button that cannot work. The order of the checks is the assertion.
        let h = Hub {
            launcher: false,
            manifest: Some(
                crate::manifest::parse(br#"{"store":{"url":"https://a.example/s"}}"#).unwrap(),
            ),
            why: None,
        };
        assert_eq!(Section::Store.status(&h), Status::NoLauncher);
    }

    #[test]
    fn the_manifest_may_rename_a_section_but_not_blank_it() {
        let named = with(r#"{"store":{"label":"COSMETICS","url":"https://a.example/s"}}"#);
        assert_eq!(Section::Store.label(&named), "COSMETICS");
        // A blank override is a hole in the nav a player cannot click with
        // any confidence, and it comes from a document we do not control.
        let blank = with(r#"{"store":{"label":"   "}}"#);
        assert_eq!(Section::Store.label(&blank), "ITEM STORE");
        assert_eq!(Section::News.label(&Hub::default()), "NEWS");
    }

    #[test]
    fn every_section_says_what_it_is_and_why_it_hands_off() {
        // The store's reason is the load-bearing one and it must actually
        // explain itself — a player who is about to be sent elsewhere to
        // spend money is owed the sentence.
        for s in Section::ALL {
            assert!(!s.blurb().is_empty());
            assert!(s.handoff_reason().contains("launcher"));
        }
        assert!(
            Section::Store.handoff_reason().contains("payment"),
            "the store must say why it will not take money in this window"
        );
    }

    #[test]
    fn settled_is_what_stops_the_screen_redrawing_forever() {
        assert!(Hub::default().settled(), "no launcher is a settled answer");
        assert!(!Hub {
            launcher: true,
            ..Hub::default()
        }
        .settled());
        assert!(with("{}").settled());
    }
}
