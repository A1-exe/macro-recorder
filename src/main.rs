use enigo::{
  Button, Coordinate, Direction, Axis,
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
  timestamp: Duration, // duration since start of recording
}

struct SharedState {
  state: State,
  recorded_events: Vec<RecordedEvent>,
  start_record_time: Option<Instant>,
  playback_thread: Option<thread::JoinHandle<()>>,
  looping: bool,
  paused_time: Duration,
  pause_instant: Option<Instant>,
  playback_start: Option<Instant>,
  recording_length: Duration,
}

impl SharedState {
  fn new() -> Self {
    SharedState {
      state: State::Idle,
      recorded_events: Vec::new(),
      start_record_time: None,
      playback_thread: None,
      looping: false,
      paused_time: Duration::ZERO,
      pause_instant: None,
      playback_start: None,
      recording_length: Duration::ZERO,
    }
  }
}

fn main() {
  let shared = Arc::new(Mutex::new(SharedState::new()));
  let s = Arc::clone(&shared);
  
  thread::spawn(move || {
    listen(move |event: Event| {
      let mut start_playback_after_unlock = false;
      let mut stop_playback_after_unlock = false;
      let mut stop_recording_after_unlock = false;
      
      {
        let mut shared = s.lock().expect("Failed to lock mutex in event listener");
        
        // Handle control keys (F1-F4)
        if let EventType::KeyPress(key) = event.event_type {
          let key_str = format!("{:?}", key);
          match key_str.as_str() {
            "F1" => {
              match shared.state {
                State::Playing => {
                  shared.state = State::Paused;
                  shared.pause_instant = Some(Instant::now());
                  print_playback_debug(&shared, "Paused");
                }
                State::Paused => {
                  if let Some(pi) = shared.pause_instant {
                    shared.paused_time += Instant::now().duration_since(pi);
                    shared.pause_instant = None;
                  }
                  shared.state = State::Playing;
                  print_playback_debug(&shared, "Resumed");
                }
                State::Recording => {
                  stop_recording_after_unlock = true;
                }
                State::Idle => {
                  if !shared.recorded_events.is_empty() {
                    start_playback_after_unlock = true;
                  } else {
                    println!("No recorded events to play.");
                  }
                }
              }
            }
            "F4" => {
              // Start recording
              start_recording(&mut shared);
            }
            "F2" => {
              if shared.state == State::Playing || shared.state == State::Paused {
                stop_playback_after_unlock = true;
              }
              if shared.state == State::Recording {
                stop_recording_after_unlock = true;
              }
              println!("Stop requested.");
            }
            "F3" => {
              shared.looping = !shared.looping;
              println!("Looping {}", if shared.looping { "enabled" } else { "disabled" });
            }
            _ => {}
          }
        }
        
        // If recording, record keys and mouse events except F1-F4
        if shared.state == State::Recording {
          if should_record_event(&event) {
            record_input_event(&mut shared, &event);
          }
        }
      }
      
      if stop_playback_after_unlock {
        stop_playback(&s);
      }
      
      if stop_recording_after_unlock {
        stop_recording(&s);
      }
      
      if start_playback_after_unlock {
        start_playback(Arc::clone(&s));
      }
    })
    .expect("Failed to listen to global events");
  });
  
  loop {
    thread::sleep(Duration::from_millis(100));
  }
}

fn should_record_event(event: &Event) -> bool {
  match event.event_type {
    EventType::KeyPress(key) | EventType::KeyRelease(key) => {
      match key {
        RdevKey::F1 | RdevKey::F2 | RdevKey::F3 | RdevKey::F4 => false,
        _ => true,
      }
    }
    _ => true,
  }
}

fn start_recording(shared: &mut SharedState) {
  if shared.state == State::Playing || shared.state == State::Paused {
    shared.state = State::Idle;
    if let Some(handle) = shared.playback_thread.take() {
      drop(handle);
    }
  }
  
  shared.recorded_events.clear();
  shared.start_record_time = Some(Instant::now());
  shared.state = State::Recording;
  println!("Recording started.");
}

fn stop_recording(s: &Arc<Mutex<SharedState>>) {
  let mut shared = s.lock().expect("Failed to lock to stop recording");
  if shared.state == State::Recording {
    shared.state = State::Idle;
    shared.start_record_time = None;
    if !shared.recorded_events.is_empty() {
      let max_time = shared
      .recorded_events
      .iter()
      .map(|e| e.timestamp)
      .max()
      .unwrap_or(Duration::ZERO);
      shared.recording_length = max_time;
    } else {
      shared.recording_length = Duration::ZERO;
    }
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
    let mut shared = s.lock().expect("Failed to lock mutex in start_playback");
    if shared.recorded_events.is_empty() {
      println!("No events to play.");
      return;
    }
    if shared.state == State::Playing {
      println!("Already playing.");
      return;
    }
    
    println!("Starting playback of {} events.", shared.recorded_events.len());
    shared.state = State::Playing;
    shared.paused_time = Duration::ZERO;
    shared.pause_instant = None;
    shared.playback_start = Some(Instant::now());
    
    print_playback_debug(&shared, "Started");
    
    shared.recorded_events.clone()
  };
  
  let s_for_thread = Arc::clone(&s);
  let handle = thread::spawn(move || {
    let mut enigo = Enigo::new(&Settings::default()).unwrap();
    
    'playback: loop {
      {
        let mut sh = s_for_thread.lock().expect("Failed to lock at iteration start");
        sh.playback_start = Some(Instant::now());
        sh.paused_time = Duration::ZERO;
        sh.pause_instant = None;
      }
      
      for evt in &events {
        loop {
          let (state, paused_time, playback_start) = {
            let shared = s_for_thread.lock().expect("Failed to lock in playback loop");
            (shared.state, shared.paused_time, shared.playback_start)
          };
          
          match state {
            State::Playing => {
              let playback_start = playback_start.unwrap();
              let effective_elapsed =
              Instant::now().duration_since(playback_start) - paused_time;
              let desired_time = evt.timestamp;
              
              if effective_elapsed < desired_time {
                let desired_ms = desired_time.as_millis() as u64;
                let now_ms = effective_elapsed.as_millis() as u64;
                if now_ms < desired_ms {
                  thread::sleep(Duration::from_millis(desired_ms - now_ms));
                }
              }
              break;
            }
            State::Paused => {
              thread::sleep(Duration::from_millis(50));
              continue;
            }
            State::Idle | State::Recording => {
              println!("Playback was stopped.");
              return;
            }
          }
        }
        
        match evt.event_type {
          EventType::MouseMove { x, y } => {
            enigo.move_mouse(x as i32, y as i32, Coordinate::Abs).unwrap();
          }
          EventType::ButtonPress(button) => {
            match button {
              rdev::Button::Left => enigo.button(Button::Left, Direction::Press).unwrap(),
              rdev::Button::Right => enigo.button(Button::Right, Direction::Press).unwrap(),
              rdev::Button::Middle => enigo.button(Button::Middle, Direction::Press).unwrap(),
              _ => ()
            };
          }
          EventType::ButtonRelease(button) => {
            match button {
              rdev::Button::Left => enigo.button(Button::Left, Direction::Release).unwrap(),
              rdev::Button::Right => enigo.button(Button::Right, Direction::Release).unwrap(),
              rdev::Button::Middle => enigo.button(Button::Middle, Direction::Release).unwrap(),
              _ => ()
            };
          }
          EventType::Wheel { delta_x, delta_y } => {
            let lines_y = delta_y as i32;
            let lines_x = delta_x as i32;
            if lines_y != 0 {
              enigo.scroll(lines_y, Axis::Vertical).unwrap();
            }
            if lines_x != 0 {
              enigo.scroll(lines_x, Axis::Horizontal).unwrap();
            }
          }
          EventType::KeyPress(key) => {
            if let Some(enigo_key) = rdev_key_to_enigo_key(key) {
              enigo.key(enigo_key, Direction::Press).unwrap();
            }
          }
          EventType::KeyRelease(key) => {
            if let Some(enigo_key) = rdev_key_to_enigo_key(key) {
              enigo.key(enigo_key, Direction::Release).unwrap();
            }
          }
        }
      }
      
      let mut sh = s_for_thread.lock().expect("Failed to lock mutex at end of playback");
      if sh.looping && sh.state == State::Playing {
        println!("Looping playback...");
        continue 'playback;
      } else {
        if sh.state == State::Playing {
          sh.state = State::Idle;
        }
        println!("Playback finished.");
      }
      
      break;
    }
  });
  
  {
    let mut shared = s.lock().expect("Failed to lock mutex after spawning playback");
    shared.playback_thread = Some(handle);
  }
}

fn stop_playback(s: &Arc<Mutex<SharedState>>) {
  {
    let mut shared = s.lock().expect("Failed to lock mutex in stop_playback");
    if shared.state == State::Playing || shared.state == State::Paused {
      shared.state = State::Idle;
      println!("Stopping playback...");
    }
  }
  
  let handle = {
    let mut shared = s.lock().expect("Failed to lock to take playback_thread");
    shared.playback_thread.take()
  };
  
  if let Some(h) = handle {
    let _ = h.join();
  }
}

fn print_playback_debug(shared: &SharedState, action: &str) {
  let recording_length_ms = shared.recording_length.as_millis();
  let simulated_time_ms = if let Some(ps) = shared.playback_start {
    if shared.state == State::Paused || shared.state == State::Playing {
      let effective = Instant::now().duration_since(ps) - shared.paused_time;
      effective.as_millis()
    } else {
      0
    }
  } else {
    0
  };
  
  println!(
    "{}. Recording length: {} ms. Current simulated time: {} ms.",
    action, recording_length_ms, simulated_time_ms
  );
}

fn rdev_key_to_enigo_key(rkey: RdevKey) -> Option<Key> {
  // Map a limited set of keys:
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