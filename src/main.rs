use anyhow::Context;
use rradio_messages::PipelineState;
use std::io::Read;

mod get_local_ip_address;
mod get_temperature;
mod lcd_screen;

type Event = rradio_messages::Event<String, String, Vec<rradio_messages::Track>>;

fn main() -> Result<(), anyhow::Error> {
    let mut lcd = lcd_screen::LcdScreen::new()
        .context("Failed to initialize LCD screen")
        .map_err(|err| {
            // Kill other screen drivers here
            err
        })?;

    lcd.write_ascii(
        lcd_screen::LCDLineNumbers::Line1,
        0,
        get_local_ip_address::get_local_ip_address().as_str(),
    );
    lcd.write_ascii(
        lcd_screen::LCDLineNumbers::Line4,
        0,
        &format!("CPU Temp {} C", get_temperature::get_cpu_temperature()),
    );

    let mut pipe_line_state: PipelineState = PipelineState::VoidPending;
    let mut volume: i32 = -1;
    let mut current_track_index: usize = 0;
    let mut current_channel: String;
    let mut station_title: String;
    let mut duration: Option<std::time::Duration> = None;
    let mut connection =
        std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, 8002)).unwrap();

    loop {
        let mut message_length_buffer = [0; 2];
        match connection.read(&mut message_length_buffer).unwrap() {
            0 => break,
            2 => (),
            _ => panic!("Weird number of bytes read"),
        }

        let message_length = u16::from_be_bytes(message_length_buffer);

        let mut buffer = vec![0; message_length as usize];

        connection.read_exact(&mut buffer).unwrap();

        //println!("length {},   {:?}", message_length, buffer);

        let event: Event = rmp_serde::from_slice(&buffer).unwrap();

        // println!("Event: {:?}", event);

        match event {
            Event::ProtocolVersion(version) => assert_eq!(version, rradio_messages::VERSION),
            Event::LogMessage(log_message) => println!("aaaaaa{:?}", log_message),
            Event::PlayerStateChanged(diff) => {
                if let Some(pipeline_state) = diff.pipeline_state {
                    pipe_line_state = pipeline_state;
                    lcd.write_volume(pipe_line_state, volume);
                }
                if let Some(current_station) = diff.current_station.into_option() {
                    duration = None;
                    if let Some(station) = current_station {
                        println!(
                            "Current Station{:?} with {} tracks",
                            station,
                            station.tracks.len()
                        );

                        if let Some(current_channel_in) = station.index {
                            current_channel = current_channel_in;
                        } else {
                            current_channel = "??".to_string();
                        }
                        lcd.write_line(
                            lcd_screen::LCDLineNumbers::Line1,
                            13,
                            format!(
                                "{Thestring:<Width$.Width$}",
                                Thestring = current_channel.as_str(),
                                Width = 13
                            )
                            .as_str(),
                        );
                        println!("current_channel {}", current_channel);

                        if let Some(title) = station.title {
                            station_title = title;
                        } else {
                            station_title = "".to_string()
                        }
                        lcd.write_line(
                            lcd_screen::LCDLineNumbers::Line2,
                            lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE as usize,
                            station_title.as_str(),
                        )
                    }
                }
                if let Some(current_track_index_in) = diff.current_track_index {
                    current_track_index = current_track_index_in;
                    println!("Current Track index: {}", current_track_index);
                }
                if let Some(current_track_tags) = diff.current_track_tags.into_option() {
                    if let Some(track_tags) = current_track_tags {
                        println!("current track tags{:?}", track_tags);
                        if let Some(ye_organisation_from_tag) = track_tags.organisation {
                            station_title = ye_organisation_from_tag;
                            lcd.write_line(
                                lcd_screen::LCDLineNumbers::Line2,
                                lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE as usize,
                                station_title.as_str(),
                            )
                        }
                        if let Some(ye_tag_title) = track_tags.title {
                            println!("ye_tag_title {}", ye_tag_title);
                        }
                    }
                }
                if let Some(volume_in) = diff.volume {
                    volume = volume_in;
                    lcd.write_volume(pipe_line_state, volume);
                }
                if let Some(buffering) = diff.buffering {
                    lcd.write_buffer_state(buffering);
                }
                if let Some(track_duration_in) = diff.track_duration.into_option() {
                    duration = track_duration_in;
                }
                if let Some(position) = diff.track_position.into_option() {
                    if let Some((duration, position)) = duration.zip(position) {
                        if position > std::time::Duration::from_secs(4) {
                            lcd.write_line(
                                lcd_screen::LCDLineNumbers::Line1,
                                13,
                                format!("{} of {}", position.as_secs(), duration.as_secs())
                                    .as_str(),
                            )
                        }
                    }
                }
            }
        }
    }
    lcd.clear();
    lcd.write_ascii(lcd_screen::LCDLineNumbers::Line1, 0, "Ending screen driver");
    lcd.write_multiline(
        lcd_screen::LCDLineNumbers::Line3,
        40,
        "Computer not shut   down",
    );
    println!("exiting screen driver");

    Ok(())
}
