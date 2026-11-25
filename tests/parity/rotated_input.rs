//! Physical button mapping follows the saved display orientation.
use crate::{
    input::ButtonEvents,
    preferences::Preferences,
    ui::{
        data::Item,
        model::{Actions, Page, UiModel, View},
    },
};

#[test]
fn all_event_masks_swap_only_plus_minus_and_keep_queue_time() {
    for mask in 0u8..=255 {
        let raw = ButtonEvents {
            pressed: mask,
            released: mask,
            short: mask,
            long: mask,
            repeat: mask,
            held: mask,
            queued_tick: 123456,
        };
        let swapped = raw.oriented(true);
        for value in [
            swapped.pressed,
            swapped.released,
            swapped.short,
            swapped.long,
            swapped.repeat,
            swapped.held,
        ] {
            assert_eq!(value & !3, mask & !3);
            assert_eq!(value & 1, (mask >> 1) & 1);
            assert_eq!((value >> 1) & 1, mask & 1);
        }
        assert_eq!(swapped.queued_tick, raw.queued_tick);
        assert_eq!(swapped.oriented(true).short, mask);
        assert_eq!(raw.oriented(false).short, mask);
    }
}

#[test]
fn physical_buttons_follow_rotation_on_dashboard_and_menu() {
    for flipped in [false, true] {
        let mut preferences = Preferences::default();
        preferences.settings.flip_screen = flipped;
        let mut model = UiModel::new(preferences);
        model.assist = 2;
        model.step_physical(
            ButtonEvents {
                short: 1,
                released: 1,
                ..Default::default()
            },
            1,
        );
        assert_eq!(model.assist, if flipped { 1 } else { 3 });
        model.step_physical(
            ButtonEvents {
                short: 2,
                released: 2,
                ..Default::default()
            },
            2,
        );
        assert_eq!(model.assist, 2);
        let physical_plus = if flipped { 2 } else { 1 };
        model.step_physical(
            ButtonEvents {
                long: physical_plus,
                held: physical_plus,
                ..Default::default()
            },
            3,
        );
        assert!(model.lights);
        model.navigation.view = View::Menu;
        model.enter(Page::Display);
        let index = (0..model.count())
            .find(|i| model.item(*i) == Item::Brightness)
            .unwrap();
        model.navigation.selected = index;
        model.step_physical(
            ButtonEvents {
                short: 4,
                released: 4,
                ..Default::default()
            },
            4,
        );
        model.step_physical(
            ButtonEvents {
                pressed: physical_plus,
                held: physical_plus,
                ..Default::default()
            },
            5,
        );
        assert_eq!(model.preferences.settings.brightness, 4);
        model.step_physical(
            ButtonEvents {
                repeat: physical_plus,
                held: physical_plus,
                ..Default::default()
            },
            6,
        );
        assert_eq!(model.preferences.settings.brightness, 5);
        model.step_physical(
            ButtonEvents {
                short: physical_plus,
                released: physical_plus,
                ..Default::default()
            },
            7,
        );
        assert_eq!(model.preferences.settings.brightness, 5);
        assert!(
            model
                .step_physical(
                    ButtonEvents {
                        long: 4,
                        held: 4,
                        ..Default::default()
                    },
                    8
                )
                .contains(Actions::SHUTDOWN)
        );
        let mut model = UiModel::new(preferences);
        model.step_physical(
            ButtonEvents {
                held: 3,
                ..Default::default()
            },
            0,
        );
        model.step_physical(
            ButtonEvents {
                held: 3,
                ..Default::default()
            },
            256,
        );
        assert_eq!(model.navigation.view, View::Menu);
        model.step_physical(
            ButtonEvents {
                released: 3,
                ..Default::default()
            },
            257,
        );
        model.navigation.selected = 0;
        model.step_physical(
            ButtonEvents {
                pressed: 1,
                held: 1,
                ..Default::default()
            },
            258,
        );
        assert_eq!(
            model.navigation.selected,
            if flipped { 1 } else { model.count() - 1 }
        );
    }
}

#[test]
fn releasing_the_key_that_rotated_the_screen_does_not_toggle_it_again() {
    let mut model = UiModel::new(Preferences::default());
    model.navigation.view = View::Menu;
    model.enter(Page::Display);
    model.navigation.selected = (0..model.count())
        .find(|i| model.item(*i) == Item::FlipScreen)
        .unwrap();
    model.step_physical(
        ButtonEvents {
            short: 4,
            released: 4,
            ..Default::default()
        },
        0,
    );
    model.step_physical(
        ButtonEvents {
            pressed: 1,
            held: 1,
            ..Default::default()
        },
        1,
    );
    assert!(!model.preferences.settings.flip_screen);
    model.step_physical(
        ButtonEvents {
            released: 1,
            short: 1,
            ..Default::default()
        },
        2,
    );
    assert!(!model.preferences.settings.flip_screen);
    assert!(
        model
            .step_physical(
                ButtonEvents {
                    short: 4,
                    released: 4,
                    ..Default::default()
                },
                3
            )
            .contains(Actions::SAVE)
    );
    assert!(!model.navigation.editing);
}
