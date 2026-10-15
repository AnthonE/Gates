//! Rebindable keys.
//!
//! Every gameplay key the player can change goes through here: an [`Action`]
//! names what the key does, [`Binds`] holds up to two binds per action, and
//! systems ask `binds.just_pressed(Action::Interact, &keys, &mouse)` instead
//! of testing a `KeyCode`. The settings screen's KEYBINDS tab edits it, and
//! the settings file keeps it as `bind.<action> = "<primary> <alt>"` lines
//! (only actions that differ from the defaults are written, so a default
//! that changes in a later build reaches players who never touched it).
//!
//! **What is not here, on purpose:** the left and right mouse buttons and
//! the wheel (too many screens give them their own meaning: placing, the
//! wheels, dragging stacks, the map), Esc (the way out of every screen),
//! the F-keys the client keeps for its own tools ([`RESERVED`]), the Shift
//! and Ctrl modifiers in combinations like Shift+K, and the digit keys of
//! screens that list numbered rows (the keypad, the hearth, pause, death).
//!
//! **Left and right modifiers are one key.** A bind on Shift, Ctrl or Alt
//! matches either side, and capturing either side stores the left one, so
//! the label can just say SHIFT.

use bevy::prelude::*;

/// One key or one mouse button.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Bind {
    Key(KeyCode),
    Mouse(MouseButton),
}

/// The right-hand twin a left modifier also answers to.
fn twin(k: KeyCode) -> Option<KeyCode> {
    match k {
        KeyCode::ShiftLeft => Some(KeyCode::ShiftRight),
        KeyCode::ControlLeft => Some(KeyCode::ControlRight),
        KeyCode::AltLeft => Some(KeyCode::AltRight),
        _ => None,
    }
}

/// What a capture stores for a pressed key: right modifiers fold into left.
pub fn normalize(k: KeyCode) -> KeyCode {
    match k {
        KeyCode::ShiftRight => KeyCode::ShiftLeft,
        KeyCode::ControlRight => KeyCode::ControlLeft,
        KeyCode::AltRight => KeyCode::AltLeft,
        k => k,
    }
}

/// Keys a player may not bind: Esc, and the function keys the client uses
/// for its own tools (F3 collision view, F4 diagnostics, F7 bug report, F11
/// fullscreen, F12 screenshot).
pub const RESERVED: [KeyCode; 6] = [
    KeyCode::Escape,
    KeyCode::F3,
    KeyCode::F4,
    KeyCode::F7,
    KeyCode::F11,
    KeyCode::F12,
];

/// The mouse buttons a bind may use. Left and right are not among them (see
/// the module header).
pub const MOUSE: [(MouseButton, &str, &str); 3] = [
    (MouseButton::Middle, "Mouse3", "MOUSE 3"),
    (MouseButton::Back, "Mouse4", "MOUSE 4"),
    (MouseButton::Forward, "Mouse5", "MOUSE 5"),
];

/// Every key a bind may use: the key, its token in the settings file, and
/// the label the screen and the prompts show. The token is the `KeyCode`
/// variant's own name, so a hand-edited file reads like the code.
pub const KEYS: [(KeyCode, &str, &str); 93] = [
    (KeyCode::KeyA, "KeyA", "A"),
    (KeyCode::KeyB, "KeyB", "B"),
    (KeyCode::KeyC, "KeyC", "C"),
    (KeyCode::KeyD, "KeyD", "D"),
    (KeyCode::KeyE, "KeyE", "E"),
    (KeyCode::KeyF, "KeyF", "F"),
    (KeyCode::KeyG, "KeyG", "G"),
    (KeyCode::KeyH, "KeyH", "H"),
    (KeyCode::KeyI, "KeyI", "I"),
    (KeyCode::KeyJ, "KeyJ", "J"),
    (KeyCode::KeyK, "KeyK", "K"),
    (KeyCode::KeyL, "KeyL", "L"),
    (KeyCode::KeyM, "KeyM", "M"),
    (KeyCode::KeyN, "KeyN", "N"),
    (KeyCode::KeyO, "KeyO", "O"),
    (KeyCode::KeyP, "KeyP", "P"),
    (KeyCode::KeyQ, "KeyQ", "Q"),
    (KeyCode::KeyR, "KeyR", "R"),
    (KeyCode::KeyS, "KeyS", "S"),
    (KeyCode::KeyT, "KeyT", "T"),
    (KeyCode::KeyU, "KeyU", "U"),
    (KeyCode::KeyV, "KeyV", "V"),
    (KeyCode::KeyW, "KeyW", "W"),
    (KeyCode::KeyX, "KeyX", "X"),
    (KeyCode::KeyY, "KeyY", "Y"),
    (KeyCode::KeyZ, "KeyZ", "Z"),
    (KeyCode::Digit0, "Digit0", "0"),
    (KeyCode::Digit1, "Digit1", "1"),
    (KeyCode::Digit2, "Digit2", "2"),
    (KeyCode::Digit3, "Digit3", "3"),
    (KeyCode::Digit4, "Digit4", "4"),
    (KeyCode::Digit5, "Digit5", "5"),
    (KeyCode::Digit6, "Digit6", "6"),
    (KeyCode::Digit7, "Digit7", "7"),
    (KeyCode::Digit8, "Digit8", "8"),
    (KeyCode::Digit9, "Digit9", "9"),
    (KeyCode::F1, "F1", "F1"),
    (KeyCode::F2, "F2", "F2"),
    (KeyCode::F3, "F3", "F3"),
    (KeyCode::F4, "F4", "F4"),
    (KeyCode::F5, "F5", "F5"),
    (KeyCode::F6, "F6", "F6"),
    (KeyCode::F7, "F7", "F7"),
    (KeyCode::F8, "F8", "F8"),
    (KeyCode::F9, "F9", "F9"),
    (KeyCode::F10, "F10", "F10"),
    (KeyCode::F11, "F11", "F11"),
    (KeyCode::F12, "F12", "F12"),
    (KeyCode::Space, "Space", "SPACE"),
    (KeyCode::Tab, "Tab", "TAB"),
    (KeyCode::Enter, "Enter", "ENTER"),
    (KeyCode::Backspace, "Backspace", "BACKSPACE"),
    (KeyCode::ShiftLeft, "ShiftLeft", "SHIFT"),
    (KeyCode::ControlLeft, "ControlLeft", "CTRL"),
    (KeyCode::AltLeft, "AltLeft", "ALT"),
    (KeyCode::CapsLock, "CapsLock", "CAPS LOCK"),
    (KeyCode::Backquote, "Backquote", "`"),
    (KeyCode::Minus, "Minus", "-"),
    (KeyCode::Equal, "Equal", "="),
    (KeyCode::BracketLeft, "BracketLeft", "L BRACKET"),
    (KeyCode::BracketRight, "BracketRight", "R BRACKET"),
    (KeyCode::Backslash, "Backslash", "\\"),
    (KeyCode::Semicolon, "Semicolon", ";"),
    (KeyCode::Quote, "Quote", "'"),
    (KeyCode::Comma, "Comma", ","),
    (KeyCode::Period, "Period", "."),
    (KeyCode::Slash, "Slash", "/"),
    (KeyCode::ArrowUp, "ArrowUp", "UP"),
    (KeyCode::ArrowDown, "ArrowDown", "DOWN"),
    (KeyCode::ArrowLeft, "ArrowLeft", "LEFT"),
    (KeyCode::ArrowRight, "ArrowRight", "RIGHT"),
    (KeyCode::Insert, "Insert", "INSERT"),
    (KeyCode::Delete, "Delete", "DELETE"),
    (KeyCode::Home, "Home", "HOME"),
    (KeyCode::End, "End", "END"),
    (KeyCode::PageUp, "PageUp", "PAGE UP"),
    (KeyCode::PageDown, "PageDown", "PAGE DOWN"),
    (KeyCode::Numpad0, "Numpad0", "NUM 0"),
    (KeyCode::Numpad1, "Numpad1", "NUM 1"),
    (KeyCode::Numpad2, "Numpad2", "NUM 2"),
    (KeyCode::Numpad3, "Numpad3", "NUM 3"),
    (KeyCode::Numpad4, "Numpad4", "NUM 4"),
    (KeyCode::Numpad5, "Numpad5", "NUM 5"),
    (KeyCode::Numpad6, "Numpad6", "NUM 6"),
    (KeyCode::Numpad7, "Numpad7", "NUM 7"),
    (KeyCode::Numpad8, "Numpad8", "NUM 8"),
    (KeyCode::Numpad9, "Numpad9", "NUM 9"),
    (KeyCode::NumpadAdd, "NumpadAdd", "NUM +"),
    (KeyCode::NumpadSubtract, "NumpadSubtract", "NUM -"),
    (KeyCode::NumpadMultiply, "NumpadMultiply", "NUM *"),
    (KeyCode::NumpadDivide, "NumpadDivide", "NUM /"),
    (KeyCode::NumpadDecimal, "NumpadDecimal", "NUM ."),
    (KeyCode::NumpadEnter, "NumpadEnter", "NUM ENTER"),
];

impl Bind {
    pub fn pressed(self, keys: &ButtonInput<KeyCode>, mouse: &ButtonInput<MouseButton>) -> bool {
        match self {
            Bind::Key(k) => keys.pressed(k) || twin(k).is_some_and(|t| keys.pressed(t)),
            Bind::Mouse(b) => mouse.pressed(b),
        }
    }

    pub fn just_pressed(
        self,
        keys: &ButtonInput<KeyCode>,
        mouse: &ButtonInput<MouseButton>,
    ) -> bool {
        match self {
            Bind::Key(k) => keys.just_pressed(k) || twin(k).is_some_and(|t| keys.just_pressed(t)),
            Bind::Mouse(b) => mouse.just_pressed(b),
        }
    }

    pub fn just_released(
        self,
        keys: &ButtonInput<KeyCode>,
        mouse: &ButtonInput<MouseButton>,
    ) -> bool {
        match self {
            Bind::Key(k) => keys.just_released(k) || twin(k).is_some_and(|t| keys.just_released(t)),
            Bind::Mouse(b) => mouse.just_released(b),
        }
    }

    /// The settings file's word for this bind, if it is one a bind may use.
    pub fn token(self) -> Option<&'static str> {
        match self {
            Bind::Key(k) => KEYS.iter().find(|e| e.0 == k).map(|e| e.1),
            Bind::Mouse(b) => MOUSE.iter().find(|e| e.0 == b).map(|e| e.1),
        }
    }

    pub fn from_token(t: &str) -> Option<Bind> {
        KEYS.iter()
            .find(|e| e.1 == t)
            .map(|e| Bind::Key(e.0))
            .or_else(|| MOUSE.iter().find(|e| e.1 == t).map(|e| Bind::Mouse(e.0)))
    }

    /// What the screen and the prompts call it.
    pub fn label(self) -> &'static str {
        match self {
            Bind::Key(k) => KEYS.iter().find(|e| e.0 == k).map_or("?", |e| e.2),
            Bind::Mouse(b) => MOUSE.iter().find(|e| e.0 == b).map_or("?", |e| e.2),
        }
    }

    /// Does this key type a character into a text field? Letters, digits,
    /// punctuation and Space do; Tab, Enter and the rest do not.
    pub fn types(self) -> bool {
        match self {
            Bind::Key(KeyCode::Space) => true,
            Bind::Key(_) => self.label().chars().count() == 1,
            Bind::Mouse(_) => false,
        }
    }

    /// May a player bind this at all?
    pub fn bindable(self) -> bool {
        match self {
            Bind::Key(k) => !RESERVED.contains(&k) && self.token().is_some(),
            Bind::Mouse(_) => self.token().is_some(),
        }
    }
}

/// What a key does. The order is the order the KEYBINDS tab lists them in,
/// and each variant's index is its row in [`SPECS`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Action {
    Forward,
    Back,
    Left,
    Right,
    Sprint,
    Crouch,
    Jump,
    FreeLook,
    Interact,
    Reload,
    Torch,
    Upgrade,
    PickUp,
    Eat,
    Drink,
    Give,
    ThrowCharge,
    SwitchOn,
    Demolish,
    ChangeSkin,
    Inventory,
    Crafting,
    Map,
    Island,
    Chat,
    TeamInvite,
    TeamAccept,
    TeamScreen,
    CrewJoin,
    CrewLeave,
    Hotbar1,
    Hotbar2,
    Hotbar3,
    Hotbar4,
    Hotbar5,
    Hotbar6,
}

/// One action's row: its file token, what the screen calls it, the group
/// heading it sits under, and its default binds.
pub struct Spec {
    pub action: Action,
    pub token: &'static str,
    pub name: &'static str,
    pub group: &'static str,
    pub default: [Option<Bind>; 2],
}

const fn k(code: KeyCode) -> Option<Bind> {
    Some(Bind::Key(code))
}

const fn spec(
    action: Action,
    token: &'static str,
    name: &'static str,
    group: &'static str,
    default: [Option<Bind>; 2],
) -> Spec {
    Spec {
        action,
        token,
        name,
        group,
        default,
    }
}

const MOVE: &str = "MOVEMENT";
const HANDS: &str = "ACTIONS";
const SCREENS: &str = "SCREENS";
const PEOPLE: &str = "TEAM AND CREW";
const BELT: &str = "HOTBAR";

pub const COUNT: usize = 36;

/// The defaults are the keys this client shipped with before binds existed.
pub static SPECS: [Spec; COUNT] = [
    spec(
        Action::Forward,
        "forward",
        "FORWARD",
        MOVE,
        [k(KeyCode::KeyW), None],
    ),
    spec(Action::Back, "back", "BACK", MOVE, [k(KeyCode::KeyS), None]),
    spec(Action::Left, "left", "LEFT", MOVE, [k(KeyCode::KeyA), None]),
    spec(
        Action::Right,
        "right",
        "RIGHT",
        MOVE,
        [k(KeyCode::KeyD), None],
    ),
    spec(
        Action::Sprint,
        "sprint",
        "SPRINT",
        MOVE,
        [k(KeyCode::ShiftLeft), None],
    ),
    spec(
        Action::Crouch,
        "crouch",
        "CROUCH",
        MOVE,
        // `Z` beside Ctrl because a browser closes the tab on Ctrl+W.
        [k(KeyCode::ControlLeft), k(KeyCode::KeyZ)],
    ),
    spec(
        Action::Jump,
        "jump",
        "JUMP",
        MOVE,
        [k(KeyCode::Space), None],
    ),
    spec(
        Action::FreeLook,
        "free_look",
        "FREE LOOK (HOLD)",
        MOVE,
        [k(KeyCode::AltLeft), None],
    ),
    spec(
        Action::Interact,
        "interact",
        "INTERACT / OPEN",
        HANDS,
        [k(KeyCode::KeyE), None],
    ),
    spec(
        Action::Reload,
        "reload",
        "RELOAD / REPAIR / ROTATE",
        HANDS,
        [k(KeyCode::KeyR), None],
    ),
    spec(
        Action::Torch,
        "torch",
        "TORCH / LOWER",
        HANDS,
        [k(KeyCode::KeyF), None],
    ),
    spec(
        Action::Upgrade,
        "upgrade",
        "UPGRADE",
        HANDS,
        [k(KeyCode::KeyU), None],
    ),
    spec(
        Action::PickUp,
        "pick_up",
        "PICK UP",
        HANDS,
        [k(KeyCode::KeyV), None],
    ),
    spec(Action::Eat, "eat", "EAT", HANDS, [k(KeyCode::KeyJ), None]),
    spec(
        Action::Drink,
        "drink",
        "DRINK",
        HANDS,
        [k(KeyCode::KeyH), None],
    ),
    spec(
        Action::Give,
        "give",
        "GIVE (HOLD)",
        HANDS,
        [k(KeyCode::KeyB), None],
    ),
    spec(
        Action::ThrowCharge,
        "throw_charge",
        "THROW A CHARGE",
        HANDS,
        [k(KeyCode::KeyX), None],
    ),
    spec(
        Action::SwitchOn,
        "switch_on",
        "LIGHT / SWITCH ON",
        HANDS,
        [k(KeyCode::KeyC), None],
    ),
    spec(
        Action::Demolish,
        "demolish",
        "DEMOLISH",
        HANDS,
        [k(KeyCode::Backspace), None],
    ),
    spec(
        Action::ChangeSkin,
        "change_skin",
        "CHANGE SKIN",
        HANDS,
        [k(KeyCode::KeyP), None],
    ),
    spec(
        Action::Inventory,
        "inventory",
        "INVENTORY",
        SCREENS,
        [k(KeyCode::Tab), k(KeyCode::KeyI)],
    ),
    spec(
        Action::Crafting,
        "crafting",
        "CRAFTING",
        SCREENS,
        [k(KeyCode::KeyQ), None],
    ),
    spec(
        Action::Map,
        "map",
        "MAP (HOLD)",
        SCREENS,
        [k(KeyCode::KeyG), None],
    ),
    spec(
        Action::Island,
        "island",
        "THE ISLAND",
        SCREENS,
        [k(KeyCode::KeyO), None],
    ),
    spec(
        Action::Chat,
        "chat",
        "CHAT",
        SCREENS,
        [k(KeyCode::KeyT), k(KeyCode::Enter)],
    ),
    spec(
        Action::TeamInvite,
        "team_invite",
        "TEAM INVITE",
        PEOPLE,
        [k(KeyCode::KeyN), None],
    ),
    spec(
        Action::TeamAccept,
        "team_accept",
        "TEAM ACCEPT",
        PEOPLE,
        [k(KeyCode::KeyY), None],
    ),
    spec(
        Action::TeamScreen,
        "team",
        "TEAM SCREEN",
        PEOPLE,
        [k(KeyCode::KeyM), None],
    ),
    spec(
        Action::CrewJoin,
        "crew_join",
        "JOIN A HEARTH'S CREW",
        PEOPLE,
        [k(KeyCode::KeyL), None],
    ),
    spec(
        Action::CrewLeave,
        "crew_leave",
        "LEAVE A CREW",
        PEOPLE,
        [k(KeyCode::KeyK), None],
    ),
    spec(
        Action::Hotbar1,
        "hotbar1",
        "SLOT 1",
        BELT,
        [k(KeyCode::Digit1), None],
    ),
    spec(
        Action::Hotbar2,
        "hotbar2",
        "SLOT 2",
        BELT,
        [k(KeyCode::Digit2), None],
    ),
    spec(
        Action::Hotbar3,
        "hotbar3",
        "SLOT 3",
        BELT,
        [k(KeyCode::Digit3), None],
    ),
    spec(
        Action::Hotbar4,
        "hotbar4",
        "SLOT 4",
        BELT,
        [k(KeyCode::Digit4), None],
    ),
    spec(
        Action::Hotbar5,
        "hotbar5",
        "SLOT 5",
        BELT,
        [k(KeyCode::Digit5), None],
    ),
    spec(
        Action::Hotbar6,
        "hotbar6",
        "SLOT 6",
        BELT,
        [k(KeyCode::Digit6), None],
    ),
];

/// The six belt slots, in order.
pub const HOTBAR: [Action; 6] = [
    Action::Hotbar1,
    Action::Hotbar2,
    Action::Hotbar3,
    Action::Hotbar4,
    Action::Hotbar5,
    Action::Hotbar6,
];

/// The controls that are not rebindable, for the KEYBINDS tab.
pub const FIXED: [(&str, &str); 12] = [
    ("LOOK", "Mouse (click to capture the pointer)"),
    (
        "USE / ATTACK",
        "Left Mouse: swings, eats, places what is in your hand",
    ),
    (
        "AIM / BUILD",
        "Right Mouse: draws a bow, aims, opens the build wheel with a plan",
    ),
    ("HOTBAR", "Scroll wheel"),
    (
        "FOUNDATION HEIGHT",
        "Shift + your RELOAD / TORCH keys with a plan",
    ),
    ("CLEAR A CREW", "Shift + your LEAVE A CREW key, twice"),
    ("SCREENSHOT", "F12"),
    (
        "DIAGNOSTICS",
        "F4: build stamp and network · F3: collision view",
    ),
    ("REPORT A BUG", "F7"),
    ("FULLSCREEN", "F11"),
    ("MENU / CLOSE", "Esc"),
    (
        "NUMBERED ROWS",
        "1 - 9 on the keypad, a hearth, the Esc and death screens",
    ),
];

impl Action {
    pub fn spec(self) -> &'static Spec {
        &SPECS[self as usize]
    }

    pub fn token(self) -> &'static str {
        self.spec().token
    }

    pub fn name(self) -> &'static str {
        self.spec().name
    }

    pub fn from_token(t: &str) -> Option<Action> {
        SPECS.iter().find(|s| s.token == t).map(|s| s.action)
    }
}

/// The player's binds: two slots per action, primary and alternate.
#[derive(Resource, Clone, PartialEq, Debug)]
pub struct Binds {
    slots: [[Option<Bind>; 2]; COUNT],
}

impl Default for Binds {
    fn default() -> Self {
        let mut slots = [[None; 2]; COUNT];
        for (i, s) in SPECS.iter().enumerate() {
            slots[i] = s.default;
        }
        Self { slots }
    }
}

impl Binds {
    pub fn slots(&self, a: Action) -> [Option<Bind>; 2] {
        self.slots[a as usize]
    }

    fn each(&self, a: Action) -> impl Iterator<Item = Bind> {
        self.slots[a as usize].into_iter().flatten()
    }

    pub fn pressed(
        &self,
        a: Action,
        keys: &ButtonInput<KeyCode>,
        mouse: &ButtonInput<MouseButton>,
    ) -> bool {
        self.each(a).any(|b| b.pressed(keys, mouse))
    }

    pub fn just_pressed(
        &self,
        a: Action,
        keys: &ButtonInput<KeyCode>,
        mouse: &ButtonInput<MouseButton>,
    ) -> bool {
        self.each(a).any(|b| b.just_pressed(keys, mouse))
    }

    pub fn just_released(
        &self,
        a: Action,
        keys: &ButtonInput<KeyCode>,
        mouse: &ButtonInput<MouseButton>,
    ) -> bool {
        self.each(a).any(|b| b.just_released(keys, mouse))
    }

    /// [`just_pressed`](Self::just_pressed) while a text field has the
    /// keyboard: only binds that do not type a character count, so a letter
    /// typed into a search box does not also close the screen.
    pub fn just_pressed_typing(
        &self,
        a: Action,
        keys: &ButtonInput<KeyCode>,
        mouse: &ButtonInput<MouseButton>,
    ) -> bool {
        self.each(a)
            .any(|b| !b.types() && b.just_pressed(keys, mouse))
    }

    /// Swallow this frame's press of `a`'s keys, so a later system does not
    /// see it too. Mouse binds are left alone (nothing downstream reads a
    /// side button as text).
    pub fn clear_just_pressed(&self, a: Action, keys: &mut ButtonInput<KeyCode>) {
        for b in self.each(a) {
            if let Bind::Key(k) = b {
                keys.clear_just_pressed(k);
                if let Some(t) = twin(k) {
                    keys.clear_just_pressed(t);
                }
            }
        }
    }

    /// Put `bind` in `a`'s `slot`. A bind can do one thing: if another
    /// action (or `a`'s other slot) already had it, it is taken from there,
    /// and the action that lost it is returned so the screen can say so.
    pub fn set(&mut self, a: Action, slot: usize, bind: Option<Bind>) -> Option<Action> {
        let mut lost = None;
        if let Some(b) = bind {
            for (i, slots) in self.slots.iter_mut().enumerate() {
                for (j, s) in slots.iter_mut().enumerate() {
                    if *s == Some(b) && !(i == a as usize && j == slot) {
                        *s = None;
                        if i != a as usize {
                            lost = Some(SPECS[i].action);
                        }
                    }
                }
            }
        }
        self.slots[a as usize][slot] = bind;
        lost
    }

    /// The label a prompt uses for `a`: its primary bind, else its
    /// alternate, else `UNBOUND`.
    pub fn key(&self, a: Action) -> &'static str {
        self.each(a).next().map_or("UNBOUND", Bind::label)
    }

    /// Rewrite the bracketed key names in a prompt written against the
    /// default binds (`[E] OPEN`, `HOLD [B]`, `[SHIFT+K]`) to the player's
    /// own. Only the part after the last `+` is a bind; `SHIFT+` in front of
    /// it is the fixed modifier. Text with no rebound key comes back as it
    /// was.
    pub fn relabel(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(open) = rest.find('[') {
            let Some(close) = rest[open..].find(']').map(|c| open + c) else {
                break;
            };
            out.push_str(&rest[..=open]);
            let inner = &rest[open + 1..close];
            let (head, last) = match inner.rfind('+') {
                Some(p) => (&inner[..=p], &inner[p + 1..]),
                None => ("", inner),
            };
            out.push_str(head);
            out.push_str(self.relabel_word(last).unwrap_or(last));
            out.push(']');
            rest = &rest[close + 1..];
        }
        out.push_str(rest);
        out
    }

    /// The player's label for whichever action has `word` as its default
    /// primary label, if that action's primary has moved.
    fn relabel_word(&self, word: &str) -> Option<&'static str> {
        let spec = SPECS
            .iter()
            .find(|s| s.default[0].is_some_and(|b| b.label().eq_ignore_ascii_case(word)))?;
        let now = self.slots(spec.action)[0];
        if now == spec.default[0] {
            return None;
        }
        Some(self.key(spec.action))
    }

    /// The settings file's `bind.*` lines: only the actions that differ from
    /// their defaults, as (action token, "primary alt") with `none` for an
    /// empty slot.
    pub fn to_lines(&self) -> Vec<(String, String)> {
        SPECS
            .iter()
            .filter(|s| self.slots(s.action) != s.default)
            .map(|s| {
                let words: Vec<&str> = self
                    .slots(s.action)
                    .iter()
                    .map(|b| b.and_then(Bind::token).unwrap_or("none"))
                    .collect();
                (s.token.to_string(), words.join(" "))
            })
            .collect()
    }

    /// Binds from the file's lines over the defaults. An unknown action, an
    /// unknown key or a reserved key keeps that action's default. Lines whose
    /// action this build does not know are returned as they came, so a save
    /// keeps them.
    pub fn from_lines(lines: &[(String, String)]) -> (Self, Vec<(String, String)>) {
        let mut binds = Self::default();
        let mut other = Vec::new();
        for (action, value) in lines {
            let Some(a) = Action::from_token(action) else {
                other.push((action.clone(), value.clone()));
                continue;
            };
            let mut slots = [None; 2];
            let mut ok = true;
            for (i, word) in value.split_whitespace().take(2).enumerate() {
                if word == "none" {
                    continue;
                }
                match Bind::from_token(word).filter(|b| b.bindable()) {
                    Some(b) => slots[i] = Some(b),
                    None => ok = false,
                }
            }
            if ok {
                // Through `set`, so a file that names one key twice cannot
                // make it do two things.
                for (i, b) in slots.into_iter().enumerate() {
                    binds.set(a, i, b);
                }
            }
        }
        (binds, other)
    }
}

/// A text node written against the default keys (`"[O] ISLAND"`). The
/// [`keyed`] system keeps its `Text` in the player's binds, so a builder can
/// write the default and not need [`Binds`] in hand.
#[derive(Component)]
pub struct Keyed(pub String);

/// Rewrite every [`Keyed`] text when the binds change, and new ones as they
/// appear.
pub fn keyed(binds: Res<Binds>, mut texts: Query<(Ref<Keyed>, &mut Text)>) {
    let all = binds.is_changed();
    for (keyed, mut text) in texts.iter_mut() {
        if !all && !keyed.is_added() {
            continue;
        }
        let want = binds.relabel(&keyed.0);
        if text.0 != want {
            text.0 = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spec_sits_at_its_own_index() {
        for (i, s) in SPECS.iter().enumerate() {
            assert_eq!(s.action as usize, i, "{}", s.token);
        }
    }

    #[test]
    fn the_defaults_bind_each_key_once_and_every_bind_is_allowed() {
        let mut seen = Vec::new();
        for s in &SPECS {
            assert!(s.default[0].is_some(), "{} has no default", s.token);
            for b in s.default.into_iter().flatten() {
                assert!(b.bindable(), "{} default {b:?} is not bindable", s.token);
                assert!(!seen.contains(&b), "{b:?} is bound twice by default");
                seen.push(b);
            }
        }
    }

    #[test]
    fn tokens_are_unique_and_round_trip() {
        for (i, a) in SPECS.iter().enumerate() {
            assert!(
                SPECS[i + 1..].iter().all(|b| b.token != a.token),
                "{}",
                a.token
            );
            assert_eq!(Action::from_token(a.token), Some(a.action));
        }
        for (code, token, _) in KEYS {
            assert_eq!(Bind::from_token(token), Some(Bind::Key(code)));
        }
        for (b, token, _) in MOUSE {
            assert_eq!(Bind::from_token(token), Some(Bind::Mouse(b)));
        }
    }

    #[test]
    fn rebinding_takes_the_key_from_whoever_had_it() {
        let mut b = Binds::default();
        let lost = b.set(Action::Interact, 0, Some(Bind::Key(KeyCode::KeyG)));
        assert_eq!(lost, Some(Action::Map));
        assert_eq!(b.slots(Action::Map)[0], None);
        assert_eq!(b.key(Action::Map), "UNBOUND");
        assert_eq!(b.key(Action::Interact), "G");
        // Into its own other slot: moved, and nobody else lost anything.
        let lost = b.set(Action::Interact, 1, Some(Bind::Key(KeyCode::KeyG)));
        assert_eq!(lost, None);
        assert_eq!(
            b.slots(Action::Interact),
            [None, Some(Bind::Key(KeyCode::KeyG))]
        );
    }

    #[test]
    fn the_file_lines_round_trip_and_defaults_write_nothing() {
        assert!(Binds::default().to_lines().is_empty());
        let mut b = Binds::default();
        b.set(Action::Interact, 0, Some(Bind::Key(KeyCode::KeyF)));
        b.set(Action::Chat, 1, None);
        b.set(Action::Give, 1, Some(Bind::Mouse(MouseButton::Back)));
        let lines = b.to_lines();
        let (back, other) = Binds::from_lines(&lines);
        assert!(other.is_empty());
        assert_eq!(back, b, "{lines:?}");
    }

    #[test]
    fn a_bad_line_keeps_the_default_and_an_unknown_action_is_kept() {
        let lines = vec![
            ("interact".to_string(), "KeyNope none".to_string()),
            ("jump".to_string(), "Escape none".to_string()),
            ("future_verb".to_string(), "KeyQ none".to_string()),
        ];
        let (b, other) = Binds::from_lines(&lines);
        assert_eq!(b.slots(Action::Interact), Action::Interact.spec().default);
        assert_eq!(b.slots(Action::Jump), Action::Jump.spec().default);
        assert_eq!(
            other,
            vec![("future_verb".to_string(), "KeyQ none".to_string())]
        );
    }

    #[test]
    fn modifiers_answer_on_either_side() {
        let mut keys = ButtonInput::<KeyCode>::default();
        let mouse = ButtonInput::<MouseButton>::default();
        keys.press(KeyCode::ShiftRight);
        assert!(Binds::default().pressed(Action::Sprint, &keys, &mouse));
        assert_eq!(normalize(KeyCode::ControlRight), KeyCode::ControlLeft);
    }

    #[test]
    fn prompts_follow_a_rebound_key_and_leave_the_rest() {
        let mut b = Binds::default();
        assert_eq!(b.relabel("[E] OPEN"), "[E] OPEN");
        b.set(Action::Interact, 0, Some(Bind::Key(KeyCode::KeyF)));
        b.set(Action::CrewLeave, 0, Some(Bind::Key(KeyCode::Semicolon)));
        assert_eq!(b.relabel("HOLD [E] HELP UP"), "HOLD [F] HELP UP");
        assert_eq!(b.relabel("[SHIFT+K] CLEAR CREW"), "[SHIFT+;] CLEAR CREW");
        assert_eq!(b.relabel("[ESC] CLOSE [M]"), "[ESC] CLOSE [M]");
        assert_eq!(b.relabel("no keys [here"), "no keys [here");
    }
}
