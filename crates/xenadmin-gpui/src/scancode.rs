//! XT make codes keyed by Linux evdev codes.
//!
//! The Rune console table in `console_view.rs` is keyed by `PhysicalKeyCode`
//! and returns the same XT values. Route A of spike S1 re-keys that table by
//! the evdev code `wl_keyboard` delivers. The table stays in this spike until
//! `xenadmin-ui` exists. It is not wired to a console view.

/// QEMU extended-key XT make code for an evdev key code, when this client
/// already sends that key.
pub fn xt_make_code(evdev: u32) -> Option<u32> {
    Some(match evdev {
        1 => 1,            // KEY_ESC
        2 => 2,            // KEY_1
        3 => 3,            // KEY_2
        4 => 4,            // KEY_3
        5 => 5,            // KEY_4
        6 => 6,            // KEY_5
        7 => 7,            // KEY_6
        8 => 8,            // KEY_7
        9 => 9,            // KEY_8
        10 => 10,          // KEY_9
        11 => 11,          // KEY_0
        12 => 12,          // KEY_MINUS
        13 => 13,          // KEY_EQUAL
        14 => 14,          // KEY_BACKSPACE
        15 => 15,          // KEY_TAB
        16 => 16,          // KEY_Q
        17 => 17,          // KEY_W
        18 => 18,          // KEY_E
        19 => 19,          // KEY_R
        20 => 20,          // KEY_T
        21 => 21,          // KEY_Y
        22 => 22,          // KEY_U
        23 => 23,          // KEY_I
        24 => 24,          // KEY_O
        25 => 25,          // KEY_P
        26 => 26,          // KEY_LEFTBRACE
        27 => 27,          // KEY_RIGHTBRACE
        28 => 28,          // KEY_ENTER
        29 => 29,          // KEY_LEFTCTRL
        30 => 0x1e,        // KEY_A
        31 => 31,          // KEY_S
        32 => 32,          // KEY_D
        33 => 33,          // KEY_F
        34 => 34,          // KEY_G
        35 => 35,          // KEY_H
        36 => 36,          // KEY_J
        37 => 37,          // KEY_K
        38 => 38,          // KEY_L
        39 => 39,          // KEY_SEMICOLON
        40 => 40,          // KEY_APOSTROPHE
        41 => 41,          // KEY_GRAVE
        42 => 42,          // KEY_LEFTSHIFT
        43 => 43,          // KEY_BACKSLASH
        44 => 44,          // KEY_Z
        45 => 45,          // KEY_X
        46 => 46,          // KEY_C
        47 => 47,          // KEY_V
        48 => 48,          // KEY_B
        49 => 49,          // KEY_N
        50 => 50,          // KEY_M
        51 => 51,          // KEY_COMMA
        52 => 52,          // KEY_DOT
        53 => 53,          // KEY_SLASH
        54 => 54,          // KEY_RIGHTSHIFT
        55 => 55,          // KEY_KPASTERISK
        56 => 56,          // KEY_LEFTALT
        57 => 57,          // KEY_SPACE
        58 => 58,          // KEY_CAPSLOCK
        59 => 59,          // KEY_F1
        60 => 60,          // KEY_F2
        61 => 61,          // KEY_F3
        62 => 62,          // KEY_F4
        63 => 63,          // KEY_F5
        64 => 64,          // KEY_F6
        65 => 65,          // KEY_F7
        66 => 66,          // KEY_F8
        67 => 67,          // KEY_F9
        68 => 68,          // KEY_F10
        69 => 69,          // KEY_NUMLOCK
        70 => 70,          // KEY_SCROLLLOCK
        71 => 71,          // KEY_KP7
        72 => 72,          // KEY_KP8
        73 => 73,          // KEY_KP9
        74 => 74,          // KEY_KPMINUS
        75 => 75,          // KEY_KP4
        76 => 76,          // KEY_KP5
        77 => 77,          // KEY_KP6
        78 => 78,          // KEY_KPPLUS
        79 => 79,          // KEY_KP1
        80 => 80,          // KEY_KP2
        81 => 81,          // KEY_KP3
        82 => 82,          // KEY_KP0
        83 => 83,          // KEY_KPDOT
        86 => 86,          // KEY_102ND
        87 => 87,          // KEY_F11
        88 => 88,          // KEY_F12
        96 => 0x9c,        // KEY_KPENTER
        97 => 0x9d,        // KEY_RIGHTCTRL
        98 => 0xb5,        // KEY_KPSLASH
        99 => 0x54,        // KEY_SYSRQ (Print Screen)
        100 => 0xb8,       // KEY_RIGHTALT
        102 => 0xc7,       // KEY_HOME
        103 => 0xc8,       // KEY_UP
        104 => 0xc9,       // KEY_PAGEUP
        105 => 0xcb,       // KEY_LEFT
        106 => 0xcd,       // KEY_RIGHT
        107 => 0xcf,       // KEY_END
        108 => 0xd0,       // KEY_DOWN
        109 => 0xd1,       // KEY_PAGEDOWN
        110 => 0xd2,       // KEY_INSERT
        111 => 0xd3,       // KEY_DELETE
        119 => 0xc6,       // KEY_PAUSE
        125 => 0xdb,       // KEY_LEFTMETA
        126 => 0xdc,       // KEY_RIGHTMETA
        127 | 139 => 0xdd, // KEY_COMPOSE and KEY_MENU
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xt_codes_match_the_rune_console_vectors() {
        assert_eq!(xt_make_code(30), Some(0x1e));
        assert_eq!(xt_make_code(106), Some(0xcd));
        assert_eq!(xt_make_code(97), Some(0x9d));
        assert_eq!(xt_make_code(99), Some(0x54));
        assert_eq!(xt_make_code(119), Some(0xc6));
    }

    #[test]
    fn left_and_right_modifiers_are_distinct() {
        assert_eq!(xt_make_code(29), Some(29));
        assert_eq!(xt_make_code(97), Some(0x9d));
        assert_eq!(xt_make_code(42), Some(42));
        assert_eq!(xt_make_code(54), Some(54));
        assert_eq!(xt_make_code(56), Some(56));
        assert_eq!(xt_make_code(100), Some(0xb8));
        assert_ne!(xt_make_code(29), xt_make_code(97));
    }

    #[test]
    fn an_unknown_evdev_code_is_not_invented() {
        assert_eq!(xt_make_code(0), None);
        assert_eq!(xt_make_code(240), None);
    }
}
