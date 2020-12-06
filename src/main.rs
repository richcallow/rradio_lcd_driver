use anyhow::Context;
use chrono::Local;
use rradio_messages::PipelineState;
use smol::io::AsyncReadExt;

mod get_local_ip_address;
mod get_temperature;
mod lcd_screen;

type Event = rradio_messages::Event<String, String, Vec<rradio_messages::Track>>;

pub enum ErrorState {
    NotKnown,
    NoError,
    /////ErrorVarious,
    NoStation,
    CdError,
    GStreamerError,
    ProgrammerError,
}

fn main() -> Result<(), anyhow::Error> {
    pretty_env_logger::init(); // options are error, warn, info, debug or trace eg RUST_LOG=info cargo run or RUST_LOG=rradio_lcd_driver=info cargo run

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
        let mut started_up = false;

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
        let mut station_type: rradio_messages::StationType = rradio_messages::StationType::CD;
        let mut got_error = false;

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
                            if !started_up {
                                lcd.write_ascii(
                                    lcd_screen::LCDLineNumbers::Line4,
                                    15,
                                    Local::now().format("%H:%M").to_string().as_str(),
                                )
                            }
                        }
                        ErrorState::CdError => println!("CD Error"),
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

            log::trace!("length {},   {:?}", message_length, buffer);

            let event: Event = rmp_serde::from_slice(&buffer).unwrap();

            log::debug!("Event: {:?}", event);

            if !started_up {
                if let Event::PlayerStateChanged(rradio_messages::PlayerStateDiff {
                    current_station: rradio_messages::OptionDiff::ChangedToSome(_),
                    ..
                }) = &event
                {
                    started_up = true;
                }
            }

            match event {
                Event::ProtocolVersion(version) => {
                    lcd.write_line(
                        lcd_screen::LCDLineNumbers::Line3,
                        lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE,
                        format!("Version {}", version).as_str(),
                    );
                    assert_eq!(version, rradio_messages::VERSION)
                }
                Event::LogMessage(log_message) => match log_message {
                    rradio_messages::LogMessage::Error(error_message) => {
                        got_error = true;
                        println!("Error message: {}", error_message);
                        let displayed_error_message = match error_message {
                            rradio_messages::Error::NoPlaylist
                            | rradio_messages::Error::InvalidTrackIndex(..) => {
                                error_state = ErrorState::ProgrammerError;
                                "Programmer Error"
                            }
                            rradio_messages::Error::PipelineError(..) => {
                                error_state = ErrorState::GStreamerError;
                                "GStreamer Error"
                            }
                            rradio_messages::Error::StationError(
                                rradio_messages::StationError::CdError(cderr),
                            ) => {
                                error_state = ErrorState::CdError;
                                println!("CD ERRRR {:?}", cderr);
                                /*match cderr {
                                    rradio_messages::StationError::CdError::CannotOpenDevice => {
                                        println!("cannot open")
                                    }
                                    _ => {}
                                }*/
                                "CD error  but what"
                            }
                            rradio_messages::Error::StationError(
                                rradio_messages::StationError::StationNotFound { index, .. },
                            ) => {
                                error_state = ErrorState::NoStation;
                                current_channel = index;
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
                                lcd.write_ascii(
                                    lcd_screen::LCDLineNumbers::Line4,
                                    0,
                                    &format!(
                                        "CPU Temp {} C",
                                        get_temperature::get_cpu_temperature()
                                    ),
                                );
                                lcd.write_time_of_day();
                                continue;
                            }
                            _ => continue,
                        };
                        println!("got error message {}", displayed_error_message);
                        lcd.write_ascii(
                            lcd_screen::LCDLineNumbers::Line1,
                            0,
                            displayed_error_message,
                        )
                    }
                },
                Event::PlayerStateChanged(diff) => {
                    got_error = false;
                    if let Some(current_station) = diff.current_station.into_option() {
                        if started_up {
                            lcd.clear()
                        };
                        duration = None;
                        error_state = ErrorState::NoError;
                        song_title = "".to_string();
                        num_of_scrolls_received = 0;
                        station_name_scroll_position = 0;
                        song_title_scroll_position = 0;
                        current_track_index = 0;
                        if let Some(station) = current_station {
                            station_type = station.source_type;
                            number_of_tracks = station
                                .tracks
                                .iter()
                                .filter(|track| !track.is_notification)
                                .count();
                            println!(
                                "Current Station{:?} with {} tracks",
                                station, number_of_tracks
                            );

                            current_channel = station.index.unwrap_or_else(|| "??".to_string());

                            let message = match station_type {
                                rradio_messages::StationType::CD => {
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
                                    "Playing CD ".to_string()
                                }
                                rradio_messages::StationType::USB => {
                                    format!("USB {}", &current_channel)
                                }
                                rradio_messages::StationType::UrlList => {
                                    format!("Station {}", &current_channel)
                                }
                                rradio_messages::StationType::FileServer => {
                                    format!("{}", &current_channel)
                                }
                            };

                            lcd.write_line(
                                lcd_screen::LCDLineNumbers::Line1,
                                lcd_screen::LCDLineNumbers::LINE1_DATA_CHAR_COUNT,
                                message.as_str(),
                            );
                            println!("current_channel {}", current_channel);
                            let st = station.title.unwrap_or_else(|| "".to_string());

                            station_title = if current_track_index == 0 {
                                st
                            } else {
                                format!("{} {}", current_track_index + 1, st)
                            };

                            if started_up {
                                lcd.write_line(
                                    lcd_screen::LCDLineNumbers::Line2,
                                    lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE,
                                    station_title.as_str(),
                                )
                            }
                        }
                    }
                    if let Some(current_track_index_in) = diff.current_track_index {
                        current_track_index = current_track_index_in;
                        if started_up {
                            match station_type {
                                rradio_messages::StationType::CD => {
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
                                }

                                _ => {}
                            }
                        }
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
                                if started_up {
                                    lcd.write_line(
                                        lcd_screen::LCDLineNumbers::Line2,
                                        lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE,
                                        station_title.as_str(),
                                    )
                                }
                            }
                            song_title = track_tags.title.unwrap_or_else(|| "".to_string());
                            println!("ye_tag_title {}", song_title);
                            if started_up {
                                lcd.write_multiline(
                                    lcd_screen::LCDLineNumbers::Line3,
                                    lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 2,
                                    song_title.as_str(),
                                );
                            }
                            num_of_scrolls_received = 0;
                            station_name_scroll_position = 0;
                            song_title_scroll_position = 0;
                        }
                    }
                    if let Some(volume_in) = diff.volume {
                        volume = volume_in;
                        lcd.write_volume(pipe_line_state, volume);
                    }
                    if let Some(pipeline_state) = diff.pipeline_state {
                        pipe_line_state = pipeline_state;
                        if let ErrorState::NoError = error_state {
                            lcd.write_volume(pipe_line_state, volume)
                        }
                    }

                    if let Some(buffering) = diff.buffering {
                        if song_title.len() <= lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE {
                            match error_state {
                                ErrorState::NoError => {
                                    if started_up {
                                        lcd.write_buffer_state(buffering);
                                    }
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
                                        let track_index = current_track_index + 1; //humans count from 1
                                        let position_secs = position.as_secs();
                                        let duration_secs = duration.as_secs();
                                        // let mut number_of_digits;
                                        let track_index_digit_count =
                                            if track_index < 10 { 1 } else { 2 };

                                        let position_secs_digit_count = match position_secs {
                                            0..=9 => 1,
                                            10..=99 => 2,
                                            100..=999 => 3,
                                            _ => 4,
                                        };

                                        let duration_secs_digit_count = match duration_secs {
                                            0..=9 => 1,
                                            10..=99 => 2,
                                            100..=999 => 3,
                                            _ => 4,
                                        };
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
