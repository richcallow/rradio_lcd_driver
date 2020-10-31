use anyhow::Context;
use rradio_messages::PipelineState;
use std::io::Read;

mod lcd_screen;

type Event = rradio_messages::Event<String, String, Vec<rradio_messages::Track>>;

fn decode_option_diff<T>(option_diff: rradio_messages::OptionDiff<T>) -> Option<Option<T>> {
    match option_diff {
        rradio_messages::OptionDiff::ChangedToSome(t) => Some(Some(t)),
        rradio_messages::OptionDiff::ChangedToNone => Some(None),
        rradio_messages::OptionDiff::NoChange => None,
    }
}

fn main() -> Result<(), anyhow::Error> {
    let mut lcd = lcd_screen::LcdScreen::new()
        .context("Failed to initialize LCD screen")
        .map_err(|err| {
            // Kill other screen drivers here
            err
        })?;

    lcd.write_ascii(
        lcd_screen::LCDLineNumbers::Line1,
        3,
        "test123456".to_string(),
    );
    lcd.write_multiline(
        lcd_screen::LCDLineNumbers::Line2,
        40,
        "test22éè123456789012345678901234567890".to_string(),
    );

    let mut pipe_line_state: PipelineState = PipelineState::VoidPending;
    let mut volume: i32 = -1;
    let mut current_track_index: usize = 0;

    let mut connection =
        std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, 8002)).unwrap();

    loop {
        let mut message_length_buffer = [0; 2];
        //lcd.write_ascii(lcd_screen::LCDLineNumbers::Line3, 8, "test why does this not work".to_string());

        match connection.read(&mut message_length_buffer).unwrap() {
            0 => break,
            2 => (),
            _ => panic!("Weird number of bytes read"),
        }

        let message_length = u16::from_be_bytes(message_length_buffer);

        let mut buffer = vec![0; message_length as usize];

        connection.read_exact(&mut buffer).unwrap();

        // println!("length {},   {:?}", message_length, buffer);

        let event: Event = rmp_serde::from_slice(&buffer).unwrap();

        // println!("Event: {:?}", event);

        match event {
            Event::ProtocolVersion(version) => assert_eq!(version, rradio_messages::VERSION),
            Event::LogMessage(log_message) => println!("{:?}", log_message),
            Event::PlayerStateChanged(diff) => {
                if let Some(pipeline_state) = diff.pipeline_state {
                    pipe_line_state = pipeline_state;
                    //print_volume(pipe_line_state, volume, lcd);
                }
                if let Some(current_station) = decode_option_diff(diff.current_station) {
                    println!("Current Station: {:?}", current_station);
                    //if let Some (station_index ) = current_station.S {}
                }
                if let Some(current_track_index_in) = diff.current_track_index {
                    current_track_index = current_track_index_in;
                    println!("Current Track index: {}", current_track_index);
                }
                if let Some(current_track_tags) = decode_option_diff(diff.current_track_tags) {
                    println!("Current Track Tags: {:?}", current_track_tags);
                }
                if let Some(volume_in) = diff.volume {
                    volume = volume_in;
                    //print_volume(pipe_line_state, volume, lcd);
                }
                if let Some(buffering) = diff.buffering {
                    println!("buffering: {}", buffering);
                }
                if let Some(track_duration) = diff.track_duration {
                    println!("track duration: {:?}", track_duration);
                }
                if let Some(track_position) = diff.track_position {
                    println!("track position: {:?}", track_position);
                }
            }
        }
    }
    fn print_volume(pipe_line_state: PipelineState, volume: i32, mut lcd: lcd_screen::LcdScreen) {
        if pipe_line_state == PipelineState::Playing && volume > 0 {
            println!("Volume {}", volume);
            lcd.write_ascii(
                lcd_screen::LCDLineNumbers::Line1,
                lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE - 7,
                format!("Vol{:>4.7}", volume),
            );
        } else {
            println!("state {:?}", pipe_line_state)
        }
    }
    Ok(())
}
