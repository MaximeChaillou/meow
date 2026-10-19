use super::play::{
    CARRY_SPEED, CATCH_COOLDOWN, CATCH_HEIGHT, CATCH_REACH, CHASE_SPEED, HOLD_MAX, HOLD_MIN,
    HUNT_LINGER, MEAL_SECONDS, NEAR_X, PIN_SECONDS, POUNCE_MIN_HEIGHT, PREY_SPEED, STARTLE_SECONDS,
    SWAT_REACH,
};
use super::*;
use crate::sprites::Prop;

const IDLE: Inputs = Inputs {
    busy: false,
    chat_open: false,
    unread: false,
};

const SCREEN: Bounds = Bounds {
    min_x: 0.0,
    max_x: 1000.0,
    max_y: 800.0,
};

const TICK: f64 = 1.0 / 60.0;
/// Shakes in tests happen above catching height, but under pouncing height.
const SHAKE_Y: f64 = (CATCH_HEIGHT + POUNCE_MIN_HEIGHT) / 2.0;

/// A cat standing still, with plenty of time before it moves on its own.
fn idle_cat(x: f64) -> Cat {
    let mut cat = Cat::new(x);
    cat.activity = Activity::Idle;
    cat.time_left = 1000.0;
    cat
}

fn tick(cat: &mut Cat, rng: &mut Rng) {
    cat.update(TICK, SCREEN, IDLE, rng);
}

fn run(cat: &mut Cat, seconds: f64) {
    let mut rng = Rng::new(1);
    for _ in 0..(seconds / TICK) as usize {
        tick(cat, &mut rng);
    }
}

/// Wiggles the pointer at (`x`, `y`) for `seconds`, then lets the cat react.
fn shake_at(cat: &mut Cat, x: f64, y: f64, seconds: f64) {
    let mut rng = Rng::new(1);
    for i in 0..(seconds / TICK) as usize {
        let wiggle = if (i / 3) % 2 == 0 { 0.0 } else { 40.0 };
        cat.see_pointer(x + wiggle, y);
        tick(cat, &mut rng);
    }
}

fn bowl_level(cat: &Cat) -> Option<u8> {
    match cat.prop()?.prop {
        Prop::Bowl(level) => Some(level),
        Prop::Mouse(_) => None,
    }
}

#[test]
fn fed_cat_eats_until_the_bowl_is_empty() {
    let mut cat = idle_cat(400.0);
    assert!(cat.feed(SCREEN));
    assert_eq!(cat.activity(), Activity::Eating);
    assert!(matches!(cat.look(IDLE).pose, Pose::Eat(_)));
    assert_eq!(bowl_level(&cat), Some(3));
    run(&mut cat, MEAL_SECONDS * 0.6);
    assert_eq!(bowl_level(&cat), Some(2));
    run(&mut cat, MEAL_SECONDS * 0.5);
    assert_ne!(cat.activity(), Activity::Eating);
    assert_eq!(cat.prop(), None);
}

#[test]
fn bowl_sits_in_front_of_the_cat() {
    let mut cat = idle_cat(400.0);
    cat.feed(SCREEN);
    assert!(cat.prop().unwrap().x > 400);
    let mut cat = idle_cat(400.0);
    cat.facing = Facing::Left;
    cat.feed(SCREEN);
    assert!((cat.prop().unwrap().x as f64) < 400.0);
}

#[test]
fn cat_at_the_edge_turns_round_to_eat() {
    let mut cat = idle_cat(SCREEN.max_x);
    assert!(cat.feed(SCREEN));
    assert_eq!(cat.facing, Facing::Left);
}

#[test]
fn chat_opening_ends_the_meal() {
    let mut cat = idle_cat(400.0);
    cat.feed(SCREEN);
    let chat = Inputs {
        chat_open: true,
        ..IDLE
    };
    cat.update(TICK, SCREEN, chat, &mut Rng::new(1));
    assert_eq!(cat.activity(), Activity::Idle);
    assert_eq!(cat.prop(), None);
}

#[test]
fn cat_cannot_be_fed_in_the_air() {
    let mut cat = idle_cat(400.0);
    cat.grab();
    assert!(!cat.feed(SCREEN));
    assert!(!cat.release_mouse(SCREEN, &mut Rng::new(1)));
}

#[test]
fn startled_cat_chases_the_mouse_until_it_escapes() {
    let mut cat = idle_cat(500.0);
    assert!(cat.release_mouse(SCREEN, &mut Rng::new(1)));
    let start = cat.prop().unwrap();
    assert!(
        start.x < 0 || start.x as f64 > SCREEN.max_x,
        "starts off screen"
    );
    assert_eq!(cat.look(IDLE).bubble, Bubble::Bang);
    run(&mut cat, STARTLE_SECONDS * 0.8);
    assert_eq!(cat.x, 500.0, "frozen for a moment");

    let mut rng = Rng::new(1);
    let mut moved = false;
    for _ in 0..(10.0 / TICK) as usize {
        let before = cat.x;
        tick(&mut cat, &mut rng);
        moved |= cat.x != before;
        assert!((cat.x - before).abs() <= CHASE_SPEED * TICK + 1e-9);
        if cat.prop().is_none() {
            break;
        }
    }
    assert!(moved, "the cat gave chase");
    assert_eq!(cat.prop(), None, "the mouse got away");
    tick(&mut cat, &mut rng);
    assert_eq!(cat.activity(), Activity::Idle);
}

#[test]
fn mouse_crosses_the_whole_screen() {
    let mut cat = idle_cat(500.0);
    cat.release_mouse(SCREEN, &mut Rng::new(1));
    let heading = cat.prop().unwrap().facing;
    let crossing = (SCREEN.max_x + CAT_WIDTH + PROP_WIDTH) / PREY_SPEED;
    run(&mut cat, crossing * 0.5);
    assert_eq!(cat.prop().unwrap().facing, heading);
    run(&mut cat, crossing * 0.6);
    assert_eq!(cat.prop(), None);
}

#[test]
fn mouse_keeps_running_while_the_cat_is_held() {
    let mut cat = idle_cat(500.0);
    cat.release_mouse(SCREEN, &mut Rng::new(1));
    let x = cat.prop().unwrap().x;
    cat.grab();
    run(&mut cat, 0.5);
    assert_ne!(cat.prop().unwrap().x, x);
}

#[test]
fn shaking_the_pointer_nearby_sets_the_cat_hunting() {
    let mut cat = idle_cat(500.0);
    let center = 500.0 + CAT_WIDTH / 2.0;
    shake_at(&mut cat, center + 20.0, SHAKE_Y, 0.6);
    assert_eq!(cat.activity(), Activity::Hunting);
}

#[test]
fn far_or_still_pointer_is_ignored() {
    let mut cat = idle_cat(500.0);
    shake_at(&mut cat, 500.0 + NEAR_X * 2.0, SHAKE_Y, 0.6);
    assert_eq!(cat.activity(), Activity::Idle);

    let mut cat = idle_cat(500.0);
    for _ in 0..30 {
        cat.see_pointer(520.0, 10.0);
        tick(&mut cat, &mut Rng::new(1));
    }
    assert_eq!(cat.activity(), Activity::Idle);
}

#[test]
fn sleeping_cat_wakes_up_to_hunt() {
    let mut cat = idle_cat(500.0);
    cat.activity = Activity::Sleeping;
    shake_at(&mut cat, 520.0, SHAKE_Y, 0.6);
    assert_eq!(cat.activity(), Activity::Hunting);
}

#[test]
fn hunting_cat_swats_at_a_low_pointer_within_reach() {
    let mut cat = idle_cat(500.0);
    let center = 500.0 + CAT_WIDTH / 2.0;
    shake_at(&mut cat, center - 20.0, SHAKE_Y, 0.6);
    cat.see_pointer(center + SWAT_REACH / 2.0, 5.0);
    tick(&mut cat, &mut Rng::new(1));
    assert!(matches!(cat.look(IDLE).pose, Pose::Swat(_)));
    assert_eq!(cat.facing, Facing::Right);
}

#[test]
fn hunting_cat_runs_to_a_distant_pointer() {
    let mut cat = idle_cat(500.0);
    let center = 500.0 + CAT_WIDTH / 2.0;
    shake_at(&mut cat, center, SHAKE_Y, 0.6);
    let x = cat.x;
    for _ in 0..30 {
        cat.see_pointer(center - 200.0, 5.0);
        tick(&mut cat, &mut Rng::new(1));
    }
    assert!(cat.x < x);
    assert_eq!(cat.facing, Facing::Left);
    assert!(matches!(cat.look(IDLE).pose, Pose::Walk(_)));
}

#[test]
fn hunting_cat_pounces_at_a_pointer_above_it() {
    let mut cat = idle_cat(500.0);
    let center = 500.0 + CAT_WIDTH / 2.0;
    shake_at(&mut cat, center, SHAKE_Y, 0.6);
    cat.see_pointer(center + 10.0, POUNCE_MIN_HEIGHT + 100.0);
    tick(&mut cat, &mut Rng::new(1));
    assert_eq!(cat.activity(), Activity::Falling);
    let mut peak: f64 = 0.0;
    while cat.activity() == Activity::Falling {
        tick(&mut cat, &mut Rng::new(1));
        peak = peak.max(cat.y);
    }
    assert!(peak > POUNCE_MIN_HEIGHT, "jumped up to {peak}");
    assert_eq!(cat.y, 0.0, "landed");
}

#[test]
fn hunt_fades_once_the_shaking_stops() {
    let mut cat = idle_cat(500.0);
    shake_at(&mut cat, 520.0, SHAKE_Y, 0.6);
    // Recent wiggles still count as a shake for a moment.
    for _ in 0..((HUNT_LINGER + 1.5) / TICK) as usize {
        cat.see_pointer(520.0, SHAKE_Y);
        tick(&mut cat, &mut Rng::new(1));
    }
    assert_ne!(cat.activity(), Activity::Hunting);
}

/// A hunting cat facing right, with the pointer resting by its paw.
fn cat_swatting_at_the_pointer() -> (Cat, (f64, f64)) {
    let mut cat = idle_cat(500.0);
    let center = 500.0 + CAT_WIDTH / 2.0;
    shake_at(&mut cat, center + 20.0, SHAKE_Y, 0.6);
    let pointer = (center + 25.0, 5.0);
    cat.see_pointer(pointer.0, pointer.1);
    (cat, pointer)
}

/// Swats until the pointer is caught; returns how long that took.
fn swat_until_caught(cat: &mut Cat, pointer: (f64, f64), rng: &mut Rng) -> Option<f64> {
    for i in 0..(30.0 / TICK) as usize {
        cat.see_pointer(pointer.0, pointer.1);
        // Keep the hunt going, as a user wiggling the pointer would.
        cat.hunt_left = HUNT_LINGER;
        tick(cat, rng);
        if cat.caught_pointer().is_some() {
            return Some(i as f64 * TICK);
        }
    }
    None
}

#[test]
fn swatting_cat_catches_the_pointer_under_its_paw() {
    let (mut cat, pointer) = cat_swatting_at_the_pointer();
    assert!(swat_until_caught(&mut cat, pointer, &mut Rng::new(9)).is_some());
    let (x, y) = cat.caught_pointer().unwrap();
    assert!(
        (x - pointer.0).abs() <= CATCH_REACH,
        "held near where it was"
    );
    assert!(y < CATCH_HEIGHT, "pinned to the ground");
    assert_eq!(cat.look(IDLE).pose, Pose::Swat(1));
}

#[test]
fn caught_pointer_is_let_go_after_a_couple_of_seconds() {
    let (mut cat, pointer) = cat_swatting_at_the_pointer();
    let mut rng = Rng::new(9);
    swat_until_caught(&mut cat, pointer, &mut rng).unwrap();
    let mut held = 0.0;
    while cat.caught_pointer().is_some() {
        tick(&mut cat, &mut rng);
        held += TICK;
    }
    assert!(
        (HOLD_MIN - TICK..=HOLD_MAX + TICK).contains(&held),
        "held {held}s"
    );
    assert_eq!(cat.activity(), Activity::Idle);

    // Satisfied for a while: shaking again does nothing.
    shake_at(&mut cat, pointer.0, SHAKE_Y, CATCH_COOLDOWN / 2.0);
    assert_ne!(cat.activity(), Activity::Hunting);
}

#[test]
fn caught_pointer_is_pinned_then_carried_off_in_the_mouth() {
    let (mut cat, pointer) = cat_swatting_at_the_pointer();
    let mut rng = Rng::new(9);
    swat_until_caught(&mut cat, pointer, &mut rng).unwrap();
    let pinned_at = cat.caught_pointer().unwrap();
    let start = cat.x;
    run(&mut cat, PIN_SECONDS * 0.8);
    assert_eq!(cat.x, start, "holds still while pinning it");
    assert_eq!(cat.caught_pointer(), Some(pinned_at));
    assert_eq!(cat.look(IDLE).pose, Pose::Swat(1));

    run(&mut cat, PIN_SECONDS * 0.4 + 0.5);
    let carried = cat.caught_pointer().expect("still has it");
    assert!(
        (cat.x - start).abs() > CARRY_SPEED * 0.3,
        "trots off with it"
    );
    assert!(carried.1 > pinned_at.1, "up in its mouth");
    assert!(matches!(cat.look(IDLE).pose, Pose::Walk(_)));

    // The pointer moves along with the cat.
    let x = cat.x;
    run(&mut cat, 0.2);
    let moved = cat.caught_pointer().unwrap();
    assert!(((moved.0 - carried.0) - (cat.x - x)).abs() < 1e-9);
}

#[test]
fn carried_pointer_turns_at_the_screen_edge() {
    let (mut cat, pointer) = cat_swatting_at_the_pointer();
    swat_until_caught(&mut cat, pointer, &mut Rng::new(9)).unwrap();
    cat.x = SCREEN.max_x - 1.0;
    run(&mut cat, PIN_SECONDS + 0.3);
    assert!(cat.x <= SCREEN.max_x);
    assert_eq!(cat.facing, Facing::Left);
}

#[test]
fn picking_the_cat_up_frees_the_pointer() {
    let (mut cat, pointer) = cat_swatting_at_the_pointer();
    swat_until_caught(&mut cat, pointer, &mut Rng::new(9)).unwrap();
    cat.grab();
    assert_eq!(cat.caught_pointer(), None);
    tick(&mut cat, &mut Rng::new(1));
    cat.release();
    // The pointer, freed, is moved well away.
    cat.see_pointer(pointer.0 + NEAR_X * 2.0, pointer.1);
    run(&mut cat, 1.0);
    assert_eq!(cat.caught_pointer(), None);
}

/// A generator whose first rolls are not all tiny, as they are for small seeds.
fn spread(seed: u64) -> Rng {
    Rng::new((seed + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// A hunting cat facing right, pouncing at a pointer hovering above it.
fn cat_pouncing(seed: u64) -> (Cat, (f64, f64)) {
    let mut cat = idle_cat(500.0);
    let center = 500.0 + CAT_WIDTH / 2.0;
    shake_at(&mut cat, center + 20.0, SHAKE_Y, 0.6);
    let pointer = (center + 30.0, POUNCE_MIN_HEIGHT + 60.0);
    cat.see_pointer(pointer.0, pointer.1);
    tick(&mut cat, &mut spread(seed));
    assert_eq!(cat.activity(), Activity::Falling, "pounced");
    (cat, pointer)
}

#[test]
fn pounce_brings_the_mouth_to_the_pointer() {
    let (mut cat, pointer) = cat_pouncing(1);
    let mut closest = f64::MAX;
    while cat.activity() == Activity::Falling {
        let (x, y) = cat.anchor(crate::sprites::MOUTH);
        closest = closest.min((x - pointer.0).hypot(y - pointer.1));
        tick(&mut cat, &mut Rng::new(1));
    }
    assert!(closest <= CATCH_REACH / 2.0, "mouth got within {closest}");
}

#[test]
fn most_pounces_snatch_the_pointer_and_land_with_it() {
    let caught = (0..40)
        .filter(|&seed| {
            let (mut cat, pointer) = cat_pouncing(seed);
            let mut rng = spread(seed);
            let mut seen = pointer;
            while cat.activity() == Activity::Falling {
                cat.see_pointer(seen.0, seen.1);
                tick(&mut cat, &mut rng);
                if let Some(held) = cat.caught_pointer() {
                    if seen == pointer {
                        assert!(held.1 > pointer.1 - CATCH_REACH, "snatched in the air");
                    }
                    // As in the app: the caught pointer goes where the mouth is.
                    seen = held;
                }
            }
            cat.caught_pointer().is_some()
        })
        .count();
    assert!((20..40).contains(&caught), "caught {caught} of 40");
}

#[test]
fn a_cat_landing_with_the_pointer_carries_it_off() {
    let (mut cat, pointer) = (0..40)
        .map(cat_pouncing)
        .find(|(cat, _)| cat.pounce_hits)
        .unwrap();
    let mut rng = Rng::new(1);
    let mut seen = pointer;
    while cat.activity() == Activity::Falling {
        cat.see_pointer(seen.0, seen.1);
        tick(&mut cat, &mut rng);
        seen = cat.caught_pointer().unwrap_or(seen);
    }
    tick(&mut cat, &mut rng);
    assert_eq!(cat.y, 0.0);
    let held = cat.caught_pointer().expect("still has it");
    let x = cat.x;
    run(&mut cat, 0.3);
    assert_ne!(cat.x, x, "trots off");
    assert_ne!(cat.caught_pointer().unwrap(), held, "pointer moves along");
}

#[test]
fn a_pointer_above_the_paw_is_never_caught() {
    let (mut cat, pointer) = cat_swatting_at_the_pointer();
    let above = (pointer.0, SHAKE_Y);
    assert_eq!(swat_until_caught(&mut cat, above, &mut Rng::new(9)), None);
    assert!(matches!(cat.look(IDLE).pose, Pose::Swat(_)));
}

#[test]
fn slow_strokes_over_the_cat_still_pet_it() {
    let mut cat = idle_cat(500.0);
    let center = 500.0 + CAT_WIDTH / 2.0;
    let mut rng = Rng::new(9);
    let mut last = center;
    for i in 0..(2.0 / TICK) as usize {
        // Two strokes a second, the length of the cat's back.
        let phase = (i as f64 * TICK * 2.0) % 1.0;
        let x = center - 60.0 + 120.0 * (1.0 - (2.0 * phase - 1.0).abs());
        cat.see_pointer(x, 30.0);
        cat.stroke(x - last);
        last = x;
        tick(&mut cat, &mut rng);
    }
    assert!(cat.is_petted());
    assert_ne!(cat.activity(), Activity::Hunting);
}

#[test]
fn shaking_over_the_cat_ends_in_a_catch() {
    // As in the app: the pointer moving over the cat window also strokes it.
    let mut cat = idle_cat(500.0);
    let center = 500.0 + CAT_WIDTH / 2.0;
    let mut rng = Rng::new(9);
    let mut held = 0.0;
    for i in 0..(5.0 / TICK) as usize {
        let wiggle = if (i / 3) % 2 == 0 { 0.0 } else { 30.0 };
        cat.see_pointer(center + wiggle, 10.0);
        cat.stroke(30.0 / 3.0);
        tick(&mut cat, &mut rng);
        if cat.caught_pointer().is_some() {
            held += TICK;
        }
        assert!(!cat.is_petted(), "shaking is not petting");
    }
    assert!(
        held >= HOLD_MIN - TICK,
        "held for {held}s, ended {:?}",
        cat.activity()
    );
}
