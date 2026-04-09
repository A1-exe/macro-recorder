use enigo::{
    Axis, Button, Coordinate, Direction,
    Enigo, Key, Keyboard, Mouse, Settings,
};
use rdev::{listen, Event, EventType, Key as RdevKey};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq, Copy, Clone)]
enum State {
    Idle,
    Recording,
    Playing,
    Paused,
}

#[derive(Debug, Clone)]
struct RecordedEvent {
    event_type: EventType,
    timestamp: Duration, // time since start of recording
}

struct SharedState {
    state: State,
    recorded_events: Vec<RecordedEvent>,
    start_record_time: Option<Instant>,
    playback_thread: Option<thread::JoinHandle<()>>,
    looping: bool,
}

impl SharedState {
    fn new() -> Self {
        Self {
            state: State::Idle,
            recorded_events: Vec::new(),
            start_record_time: None,
            playback_thread: None,
            looping: false,
        }
    }
}

fn main() {
    let shared = Arc::new(Mutex::new(SharedState::new()));
    let s = Arc::clone(&shared);

    thread::spawn(move || {
        listen(move |event: Event| {
            let mut start_playback_flag = false;
            let mut stop_playback_flag = false;
            let mut stop_recording_flag = false;

            {
                let mut shared = s.lock().unwrap();

                if let EventType::KeyPress(key) = event.event_type {
                    match key {
                        RdevKey::F1 => match shared.state {
                            State::Playing => {
                                shared.state = State::Paused;
                                println!("Paused.");
                            }
                            State::Paused => {
                                shared.state = State::Playing;
                                println!("Resumed.");
                            }
                            State::Recording => {
                                stop_recording_flag = true;
                            }
                            State::Idle => {
                                if !shared.recorded_events.is_empty() {
                                    start_playback_flag = true;
                                }
                            }
                        },
                        RdevKey::F2 => {
                            if shared.state == State::Playing
                                || shared.state == State::Paused
                            {
                                stop_playback_flag = true;
                            }
                            if shared.state == State::Recording {
                                stop_recording_flag = true;
                            }
                        }
                        RdevKey::F3 => {
                            shared.looping = !shared.looping;
                            println!(
                                "Looping {}",
                                if shared.looping {
                                    "enabled"
                                } else {
                                    "disabled"
                                }
                            );
                        }
                        RdevKey::F4 => {
                            start_recording(&mut shared);
                        }
                        _ => {}
                    }
                }

                if shared.state == State::Recording {
                    if should_record_event(&event) {
                        record_input_event(&mut shared, &event);
                    }
                }
            }

            if stop_playback_flag {
                stop_playback(&s);
            }

            if stop_recording_flag {
                stop_recording(&s);
            }

            if start_playback_flag {
                start_playback(Arc::clone(&s));
            }
        })
        .unwrap();
    });

    loop {
        thread::sleep(Duration::from_secs(1));
    }
}

fn should_record_event(event: &Event) -> bool {
    match event.event_type {
        EventType::KeyPress(key) | EventType::KeyRelease(key) => {
            !matches!(
                key,
                RdevKey::F1 | RdevKey::F2 | RdevKey::F3 | RdevKey::F4
            )
        }
        _ => true,
    }
}

fn start_recording(shared: &mut SharedState) {
    if shared.state == State::Playing || shared.state == State::Paused {
        shared.state = State::Idle;
    }

    shared.recorded_events.clear();
    shared.start_record_time = Some(Instant::now());
    shared.state = State::Recording;

    println!("Recording started.");
}

fn stop_recording(s: &Arc<Mutex<SharedState>>) {
    let mut shared = s.lock().unwrap();
    if shared.state == State::Recording {
        shared.state = State::Idle;
        shared.start_record_time = None;
        println!(
            "Recording stopped. {} events recorded.",
            shared.recorded_events.len()
        );
    }
}

fn record_input_event(shared: &mut SharedState, event: &Event) {
    if let Some(start) = shared.start_record_time {
        let elapsed = Instant::now().duration_since(start);
        shared.recorded_events.push(RecordedEvent {
            event_type: event.event_type.clone(),
            timestamp: elapsed,
        });
    }
}

fn start_playback(s: Arc<Mutex<SharedState>>) {
    let events = {
        let mut shared = s.lock().unwrap();

        if shared.recorded_events.is_empty() {
            println!("No events recorded.");
            return;
        }

        if shared.state == State::Playing {
            return;
        }

        shared.state = State::Playing;

        println!(
            "Starting playback ({} events).",
            shared.recorded_events.len()
        );

        shared.recorded_events.clone()
    };

    let s_for_thread = Arc::clone(&s);

    let handle = thread::spawn(move || {
        let mut enigo = Enigo::new(&Settings::default()).unwrap();

        loop {
            let mut last_timestamp = Duration::ZERO;

            for evt in &events {
                let delta = evt
                    .timestamp
                    .checked_sub(last_timestamp)
                    .unwrap_or(Duration::ZERO);
                last_timestamp = evt.timestamp;

                let mut remaining = delta;

                while remaining > Duration::ZERO {
                    let state = {
                        let shared = s_for_thread.lock().unwrap();
                        shared.state
                    };

                    match state {
                        State::Idle | State::Recording => {
                            println!("Playback stopped.");
                            return;
                        }
                        State::Paused => {
                            thread::sleep(Duration::from_millis(10));
                            continue;
                        }
                        State::Playing => {
                            let sleep_chunk =
                                remaining.min(Duration::from_millis(10));
                            thread::sleep(sleep_chunk);
                            remaining -= sleep_chunk;
                        }
                    }
                }

                perform_event(&mut enigo, &evt.event_type);
            }

            let looping = {
                let mut sh = s_for_thread.lock().unwrap();
                if sh.looping && sh.state == State::Playing {
                    true
                } else {
                    if sh.state == State::Playing {
                        sh.state = State::Idle;
                    }
                    false
                }
            };

            if !looping {
                println!("Playback finished.");
                break;
            }

            // println!("Looping playback...");
        }
    });

    let mut shared = s.lock().unwrap();
    shared.playback_thread = Some(handle);
}

fn stop_playback(s: &Arc<Mutex<SharedState>>) {
    let handle = {
        let mut shared = s.lock().unwrap();

        if shared.state == State::Playing
            || shared.state == State::Paused
        {
            shared.state = State::Idle;
            println!("Stopping playback...");
        }

        shared.playback_thread.take()
    };

    if let Some(h) = handle {
        let _ = h.join();
    }
}

fn perform_event(enigo: &mut Enigo, evt: &EventType) {
    match evt {
        EventType::MouseMove { x, y } => {
            enigo.move_mouse(*x as i32, *y as i32, Coordinate::Abs).unwrap();
        }
        EventType::ButtonPress(button) => match button {
            rdev::Button::Left => {
                enigo.button(Button::Left, Direction::Press).unwrap()
            }
            rdev::Button::Right => {
                enigo.button(Button::Right, Direction::Press).unwrap()
            }
            rdev::Button::Middle => {
                enigo.button(Button::Middle, Direction::Press).unwrap()
            }
            _ => {}
        },
        EventType::ButtonRelease(button) => match button {
            rdev::Button::Left => {
                enigo.button(Button::Left, Direction::Release).unwrap()
            }
            rdev::Button::Right => {
                enigo.button(Button::Right, Direction::Release).unwrap()
            }
            rdev::Button::Middle => {
                enigo.button(Button::Middle, Direction::Release).unwrap()
            }
            _ => {}
        },
        EventType::Wheel { delta_x, delta_y } => {
            if *delta_y != 0 {
                enigo.scroll(*delta_y as i32, Axis::Vertical).unwrap();
            }
            if *delta_x != 0 {
                enigo.scroll(*delta_x as i32, Axis::Horizontal).unwrap();
            }
        }
        EventType::KeyPress(key) => {
            if let Some(k) = rdev_key_to_enigo_key(*key) {
                enigo.key(k, Direction::Press).unwrap();
            }
        }
        EventType::KeyRelease(key) => {
            if let Some(k) = rdev_key_to_enigo_key(*key) {
                enigo.key(k, Direction::Release).unwrap();
            }
        }
    }
}

fn rdev_key_to_enigo_key(rkey: RdevKey) -> Option<Key> {
    use RdevKey::*;
    match rkey {
        Num0 => Some(Key::Num0),
        Num1 => Some(Key::Num1),
        Num2 => Some(Key::Num2),
        Num3 => Some(Key::Num3),
        Num4 => Some(Key::Num4),
        Num5 => Some(Key::Num5),
        Num6 => Some(Key::Num6),
        Num7 => Some(Key::Num7),
        Num8 => Some(Key::Num8),
        Num9 => Some(Key::Num9),
        KeyA => Some(Key::A),
        KeyB => Some(Key::B),
        KeyC => Some(Key::C),
        KeyD => Some(Key::D),
        KeyE => Some(Key::E),
        KeyF => Some(Key::F),
        KeyG => Some(Key::G),
        KeyH => Some(Key::H),
        KeyI => Some(Key::I),
        KeyJ => Some(Key::J),
        KeyK => Some(Key::K),
        KeyL => Some(Key::L),
        KeyM => Some(Key::M),
        KeyN => Some(Key::N),
        KeyO => Some(Key::O),
        KeyP => Some(Key::P),
        KeyQ => Some(Key::Q),
        KeyR => Some(Key::R),
        KeyS => Some(Key::S),
        KeyT => Some(Key::T),
        KeyU => Some(Key::U),
        KeyV => Some(Key::V),
        KeyW => Some(Key::W),
        KeyX => Some(Key::X),
        KeyY => Some(Key::Y),
        KeyZ => Some(Key::Z),
        ShiftLeft => Some(Key::LShift),
        ShiftRight => Some(Key::RShift),
        ControlLeft => Some(Key::LControl),
        ControlRight => Some(Key::RControl),
        Space => Some(Key::Space),
        Return => Some(Key::Return),
        Backspace => Some(Key::Backspace),
        Tab => Some(Key::Tab),
        Escape => Some(Key::Escape),
        _ => None,
    }
}