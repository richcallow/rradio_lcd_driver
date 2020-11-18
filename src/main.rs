use anyhow::Context;
use rradio_messages::PipelineState;
use smol::io::AsyncReadExt;

mod get_local_ip_address;
mod get_temperature;
mod lcd_screen;

type Event = rradio_messages::Event<String, String, Vec<rradio_messages::Track>>;

enum ErrorState {
    NotKnown,
    NoError,
    /////ErrorVarious,
    NoStation,
}

fn main() -> Result<(), anyhow::Error> {
    let mut error_state = ErrorState::NotKnown;
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

    smol::block_on(async move {
        let mut pipe_line_state: PipelineState = PipelineState::VoidPending;
        let mut volume: i32 = -1;
        let mut current_track_index: usize = 0;
        let mut current_channel: String;
        let mut station_title: String = "".to_string();
        let mut duration: Option<std::time::Duration> = None;
        let mut number_of_tracks: usize = 0;
        let mut song_title = String::new();
        let mut num_of_scrolls_received: i32 = 0;
        let mut station_name_scroll_position: usize = 0;
        let mut song_title_scroll_position: usize = 0;

        let mut connection = smol::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, 8002))
            .await
            .context("Could not connect to server")?;

        let scroll_period = std::time::Duration::from_millis(1500);
        let number_scroll_events_before_scrolling: i32 = 4000 / scroll_period.as_millis() as i32;
        let mut last_scroll_time = std::time::Instant::now();

        loop {
            let mut message_length_buffer = [0; 2];

            let next_packet = async { Ok(connection.read(&mut message_length_buffer).await) };
            let next_scroll =
                async { Err(smol::Timer::at(last_scroll_time + scroll_period).await) };

            let next_event = smol::future::race(next_packet, next_scroll);

            match next_event.await {
                Ok(bytes_read) => match bytes_read.context("Could not read buffer size")? {
                    0 => break,
                    2 => (),
                    _ => anyhow::bail!("Weird number of bytes read"),
                },
                Err(timeout_time) => {
                    match error_state {
                        ErrorState::NoStation => {
                            lcd.write_ascii(
                                lcd_screen::LCDLineNumbers::Line4,
                                0,
                                &format!("CPU Temp {} C", get_temperature::get_cpu_temperature()),
                            );
                            lcd.write_time_of_day();
                        }
                        ErrorState::NoError => {
                            if num_of_scrolls_received >= number_scroll_events_before_scrolling {
                                if song_title.len()
                                    > lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 2
                                {
                                    lcd.write_with_scroll(
                                        lcd_screen::LCDLineNumbers::Line3,
                                        lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 2,
                                        song_title.as_str(),
                                        &mut song_title_scroll_position,
                                    );
                                }
                                if station_title.len()
                                    > lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE
                                {
                                    lcd.write_with_scroll(
                                        lcd_screen::LCDLineNumbers::Line2,
                                        lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE,
                                        station_title.as_str(),
                                        &mut station_name_scroll_position,
                                    );
                                }
                            } else {
                                num_of_scrolls_received += 1; // no need to increment once we have reached the limit & this way we cannot overflow
                            }
                        }
                        _ => println!("got unexpected error state"),
                    }

                    last_scroll_time = timeout_time;
                    continue;
                }
            }

            let message_length = u16::from_be_bytes(message_length_buffer);

            let mut buffer = vec![0; message_length as usize];

            connection
                .read_exact(&mut buffer)
                .await
                .context("Could not read event")?;

            //println!("length {},   {:?}", message_length, buffer);

            let event: Event = rmp_serde::from_slice(&buffer).unwrap();

            //println!("Event: {:?}", event);

            match event {
                Event::ProtocolVersion(version) => assert_eq!(version, rradio_messages::VERSION),
                Event::LogMessage(log_message) => {
                    error_state = ErrorState::NoStation;
                    println!("aaaaaa{:?}", log_message);
                    current_channel = "??".to_string(); //todo zzz need the real channel
                    song_title = "".to_string();
                    station_title = "".to_string();
                    current_track_index = 0;
                    number_of_tracks = 0;
                    duration = None;
                    num_of_scrolls_received = 0;
                    station_name_scroll_position = 0;
                    song_title_scroll_position = 0;
                    lcd.clear();
                    lcd.write_line(
                        lcd_screen::LCDLineNumbers::Line1,
                        lcd_screen::LCDLineNumbers::LINE1_DATA_CHAR_COUNT,
                        format!("No station {}", current_channel).as_str(),
                    );
                    lcd.write_volume(pipe_line_state, volume);
                    lcd.write_ascii(
                        lcd_screen::LCDLineNumbers::Line4,
                        0,
                        &format!("CPU Temp {} C", get_temperature::get_cpu_temperature()),
                    );
                    lcd.write_time_of_day();
                }
                Event::PlayerStateChanged(diff) => {
                    if let Some(pipeline_state) = diff.pipeline_state {
                        pipe_line_state = pipeline_state;
                        lcd.write_volume(pipe_line_state, volume);
                    }
                    if let Some(current_station) = diff.current_station.into_option() {
                        lcd.clear();
                        duration = None;
                        error_state = ErrorState::NoError;
                        song_title = "".to_string();
                        num_of_scrolls_received = 0;
                        station_name_scroll_position = 0;
                        song_title_scroll_position = 0;
                        if let Some(station) = current_station {
                            number_of_tracks = station.tracks.len();
                            println!(
                                "Current Station{:?} with {} tracks",
                                station, number_of_tracks
                            );

                            current_channel = station.index.unwrap_or_else(|| "??".to_string());

                            lcd.write_line(
                                lcd_screen::LCDLineNumbers::Line1,
                                lcd_screen::LCDLineNumbers::LINE1_DATA_CHAR_COUNT,
                                current_channel.as_str(),
                            );
                            println!("current_channel {}", current_channel);
                            let st = station.title.unwrap_or_else(|| "".to_string());

                            station_title = if current_track_index == 0 {
                                st
                            } else {
                                format!("{} {}", current_track_index + 1, st)
                            };

                            lcd.write_line(
                                lcd_screen::LCDLineNumbers::Line2,
                                lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE,
                                station_title.as_str(),
                            )
                        }
                    }
                    if let Some(current_track_index_in) = diff.current_track_index {
                        current_track_index = current_track_index_in;
                        lcd.write_line(
                            lcd_screen::LCDLineNumbers::Line2,
                            lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE,
                            format!(
                                "CD track {} of {}",
                                current_track_index + 1,
                                number_of_tracks
                            )
                            .as_str(),
                        );
                        num_of_scrolls_received = 0;
                        station_name_scroll_position = 0;
                        song_title_scroll_position = 0;
                    }
                    if let Some(current_track_tags) = diff.current_track_tags.into_option() {
                        if let Some(track_tags) = current_track_tags {
                            println!(
                                "current track_tags{:?}, current_tract_index{}",
                                track_tags, current_track_index
                            );
                            if let Some(organisation_from_tag) = track_tags.organisation {
                                station_title = if current_track_index == 0 {
                                    organisation_from_tag
                                } else {
                                    format!("{} {}", current_track_index + 1, organisation_from_tag)
                                };
                                lcd.write_line(
                                    lcd_screen::LCDLineNumbers::Line2,
                                    lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE,
                                    station_title.as_str(),
                                )
                            }
                            song_title = track_tags.title.unwrap_or_else(|| "".to_string());
                            println!("ye_tag_title {}", song_title);
                            lcd.write_multiline(
                                lcd_screen::LCDLineNumbers::Line3,
                                lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 2,
                                song_title.as_str(),
                            );
                            num_of_scrolls_received = 0;
                            station_name_scroll_position = 0;
                            song_title_scroll_position = 0;
                        }
                    }
                    if let Some(volume_in) = diff.volume {
                        volume = volume_in;
                        lcd.write_volume(pipe_line_state, volume);
                    }
                    if let Some(buffering) = diff.buffering {
                        if song_title.len() <= lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE {
                            match error_state {
                                ErrorState::NoError => {
                                    lcd.write_buffer_state(buffering);
                                }
                                _ => {}
                            }
                        }
                    }
                    if let Some(track_duration_in) = diff.track_duration.into_option() {
                        duration = track_duration_in;
                    }
                    if let Some(position) = diff.track_position.into_option() {
                        if let Some((duration, position)) = duration.zip(position) {
                            if pipe_line_state == rradio_messages::PipelineState::Playing {
                                match error_state {
                                    ErrorState::NoError => {
                                        lcd.write_line(
                                            lcd_screen::LCDLineNumbers::Line1,
                                            lcd_screen::LCDLineNumbers::LINE1_DATA_CHAR_COUNT
                                                as usize,
                                            format!(
                                                "{}, {} of {}",
                                                current_track_index + 1, //humans count from 1
                                                position.as_secs(),
                                                duration.as_secs()
                                            )
                                            .as_str(),
                                        )
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }
        lcd.clear(); //we are ending the program if we get to here
        lcd.write_ascii(lcd_screen::LCDLineNumbers::Line1, 0, "Ending screen driver");
        lcd.write_multiline(
            lcd_screen::LCDLineNumbers::Line3,
            40,
            "Computer not shut   down",
        );
        println!("exiting screen driver");

        Ok(())
    })
}
