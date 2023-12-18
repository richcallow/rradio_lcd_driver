//use std::thread::current;

use anyhow::Context;
use chrono::Local;

use futures_util::StreamExt;
//use rradio_messages::{ArcStr, CdError, Event, PipelineState, PlayerStateDiff};
use rradio_messages::{ArcStr, Event, LatestError, PipelineState};
use rradio_messages::{PingTarget, PingTimes};

mod get_local_ip_address;
mod lcd;

use lcd::{LINE1_DATA_CHAR_COUNT_USIZE, NUM_CHARACTERS_PER_LINE};

mod try_to_kill_earlier_versions_of_lcd_screen_driver;

#[derive(PartialEq, Debug)]
pub enum ErrorState {
    NotKnown,
    NoError,
    NoStation,
    CdError,
    CdEjectError,
    UsbOrSambaError,
    GStreamerError,
    ProgrammerError,
    UPnPError,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), anyhow::Error> {
    //pretty_env_logger::init(); // options are error, warn, info, debug or trace eg RUST_LOG=info cargo run or RUST_LOG=rradio_lcd_driver=info cargo run
    try_to_kill_earlier_versions_of_lcd_screen_driver::try_to_kill_earlier_versions_of_lcd_screen_driver();

    let mut no_connection_counter = 0;

    let mut lcd = lcd::Lc::new()?; // open the LCD screen & panic if it fails;

    lcd.write_ascii(
        lcd::LineNum::Line1,
        0,
        get_local_ip_address::get_local_ip_address().as_str(),
    );

    lcd.write_ascii(lcd::LineNum::Line1, 13, "\x00\x01 \x02\x03 \x04\x05");

    lcd.write_multiline(
        lcd::LineNum::Line2,
        NUM_CHARACTERS_PER_LINE * 2,
        format!("Expecting version   {} of rradio", rradio_messages::VERSION).as_str(), // the spaces are intentional
    );
    /*lcd.write_multiline(               // write the 8 bespoke characters & other test characters to the screen.
    lc::LineNum::Line1,             // this line is only needed for test purposes.
    40,
    "\x00 \x01 \x02 \x03 \x04  \x05  \x06  \x07 aéñ€èüπÆÇ∞n°qq");
    */

    let mut started_up = false;
    let mut error_state = ErrorState::NotKnown;
    let mut pipe_line_state = PipelineState::Null;
    let mut volume = -1_i32;
    let mut muted = false;
    let mut current_track_index: usize = 0;
    let mut current_channel;
    let mut last_error_channel = String::new(); // an invalid value intentionally
    let mut last_error_channel_not_changed = false;
    let mut line2_text = String::new();
    let mut duration: Option<std::time::Duration> = None;
    let mut number_of_tracks = 0;
    let mut song_title = String::new();
    let mut num_of_scrolls_received: i32 = 0;
    let mut line2_text_scroll_position: usize = 0;
    let mut song_title_scroll_position: usize = 0;
    let mut organisation = String::new();
    let mut artist = String::new();
    let mut album = String::new();
    let mut station_type: rradio_messages::StationType = rradio_messages::StationType::CD;
    let mut station_title = String::new();
    let mut station_change_time;
    let mut got_station = false;
    let scroll_period = tokio::time::Duration::from_millis(1600);
    let number_scroll_events_before_scrolling: i32 = 3000 / scroll_period.as_millis() as i32;

    let mut last_scroll_time = tokio::time::Instant::now();
    let mut error_message_output = false;
    let mut pause_before_playing = 0;
    let mut show_temparature_instead_of_gateway_ping = false;

    let rradio_events = loop {
        match tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, 8002)).await {
            Ok(stream) => {
                lcd.clear(); //clear out the error message
                lcd.write_ascii(
                    lcd::LineNum::Line1,
                    0,
                    get_local_ip_address::get_local_ip_address().as_str(),
                );
                // we have got a message, but we do not know if it is valid, so we try & decode it
                break rradio_messages::Event::decode_from_stream(tokio::io::BufReader::new(
                    stream,
                ))
                .await
                .map_err(|err| {
                    match &err {
                        rradio_messages::BadRRadioHeader::FailedToReadHeader(_io_error) => {
                            lcd.write_multiline(
                                // it was a really bad error, so write it to the LCD screen.
                                // it was so bad we might as well write to the entire screen
                                lcd::LineNum::Line1,
                                lcd::NUM_CHARACTERS_PER_LINE * 4,
                                err.to_string().as_str(),
                            );
                            println!("{:?}", err.to_string().as_str());
                        }
                        rradio_messages::BadRRadioHeader::HeaderMismatch {
                            expected: _,
                            actual,
                        } => match std::str::from_utf8(&actual[..]) {
                            // the convert to UTF8 assumes that the characters are ASCII
                            Err(_) => {
                                lcd.write_multiline(
                                    // oops they were not ASCII so report the problem
                                    lcd::LineNum::Line1,
                                    lcd::NUM_CHARACTERS_PER_LINE,
                                    "Bad RRadio Header",
                                );
                                lcd.write_multiline(
                                    lcd::LineNum::Line2,
                                    lcd::NUM_CHARACTERS_PER_LINE,
                                    "Not UTF-8",
                                );
                                lcd.write_multiline(
                                    lcd::LineNum::Line3,
                                    lcd::NUM_CHARACTERS_PER_LINE * 2,
                                    &rradio_messages::DisplayApiHeader(&actual[..]).to_string(),
                                );
                            }
                            Ok(actual_str) => match actual_str.strip_prefix("rradio-messages_") {
                                // it was ASCII; it should start with the string "rradio-messages_"
                                // so try & remove the string, which of course could fail
                                None => {
                                    // it did fail, so it could not have been a rradio message
                                    lcd.write_multiline(
                                        lcd::LineNum::Line1,
                                        lcd::NUM_CHARACTERS_PER_LINE,
                                        "Not RRadio",
                                    );
                                    lcd.write_multiline(
                                        lcd::LineNum::Line2,
                                        lcd::NUM_CHARACTERS_PER_LINE * 3,
                                        &rradio_messages::DisplayApiHeader(&actual[..]).to_string(),
                                    );
                                }
                                Some(version) => {
                                    // the message was from rradio, but the version was wrong.
                                    lcd.write_multiline(
                                        lcd::LineNum::Line1,
                                        lcd::NUM_CHARACTERS_PER_LINE,
                                        "Version Mismatch",
                                    );
                                    lcd.write_multiline(
                                        lcd::LineNum::Line2,
                                        lcd::NUM_CHARACTERS_PER_LINE,
                                        &format!("Local:  {}", rradio_messages::VERSION),
                                    );
                                    lcd.write_multiline(
                                        lcd::LineNum::Line3,
                                        lcd::NUM_CHARACTERS_PER_LINE,
                                        &format!("Remote: {}", version.trim_end()),
                                    );
                                }
                            },
                        },
                    };
                    err
                })
                .context("Header mismatch")?;
            }
            Err(error) => {
                no_connection_counter += 1;
                println!(
                    "Connnection count{}: Connection Error: {:?}",
                    no_connection_counter, error
                );
                // line 3 contains the version number so cannot use it.
                lcd.write_temperature_and_strength(lcd::LineNum::Line4);
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                // Wait for 1000ms
            }
        }
    };
    tokio::pin!(rradio_events);

    const PING_TIME_NONE: &str = "Ping Time None";

    station_change_time = tokio::time::Instant::now(); //now that we have a connection, note when we start
    loop {
        // fetch the next rradio event, or scroll on timeout
        let timeout_time = last_scroll_time + scroll_period;
        let next_rradio_event =
            match tokio::time::timeout_at(timeout_time, rradio_events.next()).await {
                Ok(None) => break, // No more rradio events, so shutdown
                Ok(Some(event)) => event?,
                Err(_) => {
                    ////////////println!("error_state{:?}", error_state);
                    match error_state {
                        ErrorState::NoStation => {
                            lcd.write_ascii(
                                lcd::LineNum::Line2,
                                0,
                                get_local_ip_address::get_local_ip_address().as_str(),
                            );
                            lcd.write_date_and_time_of_day_line3();

                            if last_error_channel_not_changed {
                                lcd.write_multiline(
                                    lcd::LineNum::Line4,
                                    lcd::NUM_CHARACTERS_PER_LINE,
                                    "\x00 \x01 \x02 \x03 \x04\x05\x06\x07ñäöü~ÆÇ",
                                )
                            } else {
                                lcd.write_temperature_and_strength(lcd::LineNum::Line4);
                            }
                        }
                        ErrorState::NoError => {
                            if num_of_scrolls_received >= number_scroll_events_before_scrolling {
                                if song_title.len() > lcd::NUM_CHARACTERS_PER_LINE * 2 {
                                    lcd.write_with_scroll(
                                        lcd::LineNum::Line3,
                                        lcd::NUM_CHARACTERS_PER_LINE * 2,
                                        song_title.as_ref(),
                                        &mut song_title_scroll_position,
                                    );
                                } else if song_title.is_empty() && started_up {
                                    // we have space to write the temperature
                                    lcd.write_temperature_and_strength(lcd::LineNum::Line3)
                                }
                                if line2_text.len() > NUM_CHARACTERS_PER_LINE {
                                    lcd.write_with_scroll(
                                        lcd::LineNum::Line2,
                                        lcd::NUM_CHARACTERS_PER_LINE,
                                        line2_text.as_str(),
                                        &mut line2_text_scroll_position,
                                    );
                                }
                            } else {
                                num_of_scrolls_received += 1; // no need to increment once we have reached the limit & this way we cannot overflow
                            }
                        }
                        ErrorState::NotKnown => lcd.write_multiline(
                            lcd::LineNum::Line1,
                            lcd::NUM_CHARACTERS_PER_LINE * 4,
                            "Error state: unknown.",
                        ),
                        ErrorState::CdError => {
                            if !error_message_output {
                                lcd.write_multiline(
                                    lcd::LineNum::Line1,
                                    lcd::NUM_CHARACTERS_PER_LINE * 4,
                                    "Error state: CD Error",
                                )
                            }
                        }
                        ErrorState::CdEjectError => lcd.write_multiline(
                            lcd::LineNum::Line1,
                            lcd::NUM_CHARACTERS_PER_LINE * 4,
                            "Error state CD eject error",
                        ),
                        ErrorState::UPnPError => lcd.write_multiline(
                            lcd::LineNum::Line1,
                            lcd::NUM_CHARACTERS_PER_LINE * 4,
                            "UPnP error",
                        ),
                        ErrorState::UsbOrSambaError => lcd.write_multiline(
                            lcd::LineNum::Line1,
                            lcd::NUM_CHARACTERS_PER_LINE * 4,
                            "Error state: USB or server Error",
                        ),
                        ErrorState::ProgrammerError => lcd.write_multiline(
                            lcd::LineNum::Line1,
                            lcd::NUM_CHARACTERS_PER_LINE * 4,
                            "Error state:: Programmer error",
                        ),
                        ErrorState::GStreamerError => println!(
                            "Not writing the error to screen as it is reported as last error"
                        ), //_ => println!("got unexpected error state {:?}", error_state),
                    }
                    last_scroll_time = timeout_time;
                    continue;
                }
            };

        if !started_up {
            if let Event::PlayerStateChanged(rradio_messages::PlayerStateDiff {
                current_station: Some(rradio_messages::CurrentStation::PlayingStation { .. }),
                ..
            }) = next_rradio_event
            {
                started_up = true;
            }
        }

        match next_rradio_event {
            Event::PlayerStateChanged(player_state_difference) => {
                println!("player_state_differance= {:?}", player_state_difference);

                if let Some(lastest_error_as_option) = player_state_difference.latest_error {
                    match lastest_error_as_option {
                        Some(latest_error) => {
                            error_state = ErrorState::GStreamerError;
                            lcd.write_multiline(
                                lcd::LineNum::Line1,
                                NUM_CHARACTERS_PER_LINE * 4,
                                format!("latest_error{}", latest_error.error.to_string()).as_str(),
                            )
                        }
                        None => {
                            println!("Latest error was none!");
                        }
                    }
                }

                if let Some(pipeline_state) = player_state_difference.pipeline_state {
                    print!("pipeline state is {}", pipeline_state);
                    pipe_line_state = pipeline_state;
                    if let ErrorState::NoError = error_state {
                        lcd.write_volume(pipe_line_state, muted, volume)
                    }
                }
                if let Some(is_muted) = player_state_difference.is_muted {
                    muted = is_muted;
                    lcd.write_volume(pipe_line_state, muted, volume);
                }

                if started_up && station_change_time.elapsed() > std::time::Duration::from_secs(6) {
                    if let Some(ref ping_times) = player_state_difference.ping_times {
                        show_temparature_instead_of_gateway_ping =
                            !show_temparature_instead_of_gateway_ping;
                        let ping_message = match ping_times {
                            PingTimes::FinishedPingingRemote { gateway_ping: _ }
                                if show_temparature_instead_of_gateway_ping =>
                            {
                                format!("CPU temp{:>3}C", lcd.get_cpu_temperature())
                            }
                            PingTimes::Gateway(Ok(gateway_ping))
                            | PingTimes::GatewayAndRemote {
                                gateway_ping, // this branch matches local pings that did not give an error
                                remote_ping: _,
                                latest: PingTarget::Gateway,
                            }
                            | PingTimes::FinishedPingingRemote { gateway_ping } => {
                                if gateway_ping.as_nanos() < 9_999_999 {
                                    format!(
                                        "LocPing{:.width$}ms",
                                        gateway_ping.as_micros() as f32 / 1000.0,
                                        width = 1
                                    )
                                } else {
                                    format!("LocPing{:>3}ms", gateway_ping.as_nanos() / 1_000_000)
                                }
                            }
                            .to_string(),
                            PingTimes::GatewayAndRemote {
                                gateway_ping: _, //this branch matches remote pings that did not give an error
                                remote_ping: Ok(remote_ping),
                                latest: PingTarget::Remote,
                            } => {
                                if remote_ping.as_nanos() < 9_999_999 {
                                    format!(
                                        "RemPing{:.width$}ms",
                                        remote_ping.as_micros() as f32 / 1000.0,
                                        width = 1
                                    )
                                } else {
                                    format!("RemPing{:>3}ms", remote_ping.as_nanos() / 1_000_000)
                                }
                            }
                            PingTimes::None => PING_TIME_NONE.to_string(),
                            PingTimes::BadUrl => "Bad URL".to_string(),
                            PingTimes::Gateway(Err(gateway_error)) => ({
                                match gateway_error {
                                    rradio_messages::PingError::FailedToRecieveICMP => {
                                        "LPing Rx fail"
                                    } // OS raised error when receiving ICMP message
                                    rradio_messages::PingError::DestinationUnreachable => {
                                        "LDest Unreach"
                                    } // Ping response reported as "Destination Unreachable"
                                    rradio_messages::PingError::Timeout => "LPing NoReply",
                                    rradio_messages::PingError::FailedToSendICMP => "LPing Tx Fail",
                                    rradio_messages::PingError::Dns => "LPing DNS err",
                                }
                            })
                            .to_string(),
                            PingTimes::GatewayAndRemote {
                                gateway_ping: _, //this brach matches remote ping that failed
                                remote_ping: Err(remote_error),
                                latest: PingTarget::Remote,
                            } => {
                                match remote_error {
                                    rradio_messages::PingError::FailedToRecieveICMP => {
                                        "LPing: Rx fail"
                                    }
                                    rradio_messages::PingError::DestinationUnreachable => {
                                        "RDest Unreach"
                                    }
                                    rradio_messages::PingError::Timeout => "RPing NoReply",
                                    rradio_messages::PingError::FailedToSendICMP => "LPing Tx fail",
                                    rradio_messages::PingError::Dns => "RPing DNS err",
                                }
                            }
                            .to_string(),
                        };
                        if ping_message != PING_TIME_NONE
                            && error_state != ErrorState::GStreamerError
                        {
                            lcd.write_multiline(
                                lcd::LineNum::Line1,
                                LINE1_DATA_CHAR_COUNT_USIZE,
                                &ping_message,
                            )
                        };
                    }
                }

                if let Some(pause_before_playing_in) = player_state_difference.pause_before_playing
                {
                    if let Some(pause_before_playing_as_duration) = pause_before_playing_in {
                        pause_before_playing = pause_before_playing_as_duration.as_secs();
                    } else {
                        pause_before_playing = 0;
                    }
                }

                if let Some(current_track_index_as_usize) =
                    player_state_difference.current_track_index
                {
                    current_track_index = current_track_index_as_usize;
                    lcd.write_all_line_2(&format!(
                        "CD track {} of {}",
                        current_track_index + 1,
                        number_of_tracks
                    ));
                };
                println!("current_track_index {}", current_track_index);
                if !started_up {
                    if let Some(ping_times) = player_state_difference.ping_times {
                        show_temparature_instead_of_gateway_ping =
                            !show_temparature_instead_of_gateway_ping;
                        let ping_message = match ping_times {
                            PingTimes::FinishedPingingRemote { gateway_ping: _ }
                                if show_temparature_instead_of_gateway_ping =>
                            {
                                format!("CPU temp{:>3}C", lcd.get_cpu_temperature())
                            }
                            PingTimes::Gateway(Ok(gateway_ping))
                            | PingTimes::GatewayAndRemote {
                                gateway_ping, // this branch matches local pings that did not give an error
                                remote_ping: _,
                                latest: PingTarget::Gateway,
                            }
                            | PingTimes::FinishedPingingRemote { gateway_ping } => {
                                format!(
                                    "Local ping {:.width$}ms",
                                    gateway_ping.as_micros() as f32 / 1000.0,
                                    width = 1
                                )
                            }
                            .to_string(),
                            PingTimes::GatewayAndRemote {
                                gateway_ping: _, //this branch matches remote pings that did not give an error
                                remote_ping: Ok(remote_ping),
                                latest: PingTarget::Remote,
                            } => {
                                format!(
                                    "Remote ping {:.width$}ms",
                                    remote_ping.as_micros() as f32 / 1000.0,
                                    width = 1
                                )
                            }
                            PingTimes::None => PING_TIME_NONE.to_string(),
                            PingTimes::BadUrl => "Bad URL".to_string(),
                            PingTimes::Gateway(Err(gateway_error)) => ({
                                match gateway_error {
                                    rradio_messages::PingError::FailedToRecieveICMP => {
                                        "Local ping Rx fail"
                                    } // OS raised error when receiving ICMP message
                                    rradio_messages::PingError::DestinationUnreachable => {
                                        "Loc Dest Unreachable"
                                    } // Ping response reported as "Destination Unreachable"
                                    rradio_messages::PingError::Timeout => "Local ping: No reply",
                                    rradio_messages::PingError::FailedToSendICMP => {
                                        "Local ping: Tx Fail"
                                    }
                                    rradio_messages::PingError::Dns => "Local ping DNS error",
                                }
                            })
                            .to_string(),

                            PingTimes::GatewayAndRemote {
                                gateway_ping: _, //this brach matches remote ping that failed
                                remote_ping: Err(remote_error),
                                latest: PingTarget::Remote,
                            } => {
                                match remote_error {
                                    rradio_messages::PingError::FailedToRecieveICMP => {
                                        "LPing: Rx fail"
                                    }
                                    rradio_messages::PingError::DestinationUnreachable => {
                                        "RDest Unreach"
                                    }
                                    rradio_messages::PingError::Timeout => "RPing NoReply",
                                    rradio_messages::PingError::FailedToSendICMP => "LPing Tx fail",
                                    rradio_messages::PingError::Dns => "RPing DNS err",
                                }
                            }
                            .to_string(),
                        };
                        if ping_message != PING_TIME_NONE {
                            lcd.write_multiline(
                                lcd::LineNum::Line2,
                                lcd::NUM_CHARACTERS_PER_LINE,
                                &ping_message,
                            );
                        };

                        if !started_up {
                            lcd.write_date_and_time_of_day_line3();
                            lcd.write_temperature_and_time_to_line4();
                        }
                    }
                }

                if let Some(volume_in) = player_state_difference.volume {
                    volume = volume_in;
                    lcd.write_volume(pipe_line_state, muted, volume);
                }

                if let Some(track_duration_in) = player_state_difference.track_duration {
                    duration = track_duration_in;
                }

                if let Some(position) = player_state_difference.track_position {
                    if let Some((duration, position)) = duration.zip(position) {
                        if got_station
                            && pipe_line_state == rradio_messages::PipelineState::Playing
                            && error_state == ErrorState::NoError
                        {
                            let track_index = current_track_index + 1; // humans count from 1
                            let position_secs = position.as_secs();
                            let duration_secs = duration.as_secs();
                            // let mut number_of_digits;
                            let track_index_digit_count = if track_index < 10 { 1 } else { 2 };

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
                            let number_of_digits = track_index_digit_count
                                + position_secs_digit_count
                                + duration_secs_digit_count;
                            let message = match number_of_digits {
                                0..=7 => format!(
                                    "{}: {} of {}",
                                    track_index, position_secs, duration_secs
                                ),
                                8 => format!(
                                    "{}:{} of {}",
                                    track_index, position_secs, duration_secs
                                ),
                                9 => {
                                    format!("{}:{}of {}", track_index, position_secs, duration_secs)
                                }
                                10 => {
                                    format!("{}: {}of{}", track_index, position_secs, duration_secs)
                                }
                                _ => format!("{}: {}", track_index, position_secs),
                            };
                            if (position_secs >= 2) | (current_track_index > 0) {
                                //wait 2 seconds so that people can read what comes before for the first track
                                lcd.write_multiline(
                                    lcd::LineNum::Line1,
                                    LINE1_DATA_CHAR_COUNT_USIZE,
                                    message.as_str(),
                                );
                            }
                        }
                    }
                }
                if let Some(current_station) = player_state_difference.current_station {
                    if started_up {
                        got_station = true;
                        error_state = ErrorState::NoError;
                        lcd.clear();
                    }
                    lcd.write_volume(pipe_line_state, muted, volume);
                    station_change_time = tokio::time::Instant::now();
                    error_state = ErrorState::NoError;
                    num_of_scrolls_received = 0;
                    line2_text_scroll_position = 0;
                    song_title_scroll_position = 0;
                    song_title = "".to_string();
                    // do NOT set current_track_index to zero here. It is seemingly set whenever the station is changed.
                    //println!("Current station {:?}", current_station);

                    match current_station {
                        rradio_messages::CurrentStation::NoStation => {}
                        rradio_messages::CurrentStation::FailedToPlayStation { error } => {
                            match error {
                                rradio_messages::StationError::StationNotFound {
                                    index,
                                    directory: _,
                                } => {
                                    error_state = ErrorState::NoStation;
                                    current_channel = index.to_string();
                                    song_title = String::new();
                                    line2_text = "".to_string();
                                    current_track_index = 0;
                                    station_title = String::new();
                                    //number_of_tracks = 0;
                                    duration = None;
                                    num_of_scrolls_received = 0;
                                    line2_text_scroll_position = 0;
                                    song_title_scroll_position = 0;
                                    lcd.clear();
                                    error_message_output = false;
                                    lcd.write_multiline(
                                        lcd::LineNum::Line1,
                                        NUM_CHARACTERS_PER_LINE,
                                        format!("No station {}", current_channel).as_str(),
                                    );
                                    lcd.write_date_and_time_of_day_line3();

                                    lcd.write_temperature_and_strength(lcd::LineNum::Line4);
                                    lcd.write_ascii(
                                        lcd::LineNum::Line4,
                                        17,
                                        Local::now().format("%a").to_string().as_str(),
                                    );
                                    last_error_channel_not_changed =
                                        last_error_channel == current_channel;
                                    last_error_channel = current_channel;
                                    continue;
                                }
                                rradio_messages::StationError::UPnPError(error_string) => {
                                    error_state = ErrorState::UPnPError;
                                    lcd.write_multiline(
                                        lcd::LineNum::Line1,
                                        lcd::NUM_CHARACTERS_PER_LINE * 4,
                                        format!("upnp error {}", error_string.to_string()).as_str(),
                                    )
                                }
                                rradio_messages::StationError::MountError(mount_error) => {
                                    error_state = ErrorState::NotKnown;
                                    lcd.write_multiline(
                                        lcd::LineNum::Line1,
                                        lcd::NUM_CHARACTERS_PER_LINE * 4,
                                        format!("mount error {:?}", mount_error).as_str(),
                                    )
                                }
                                rradio_messages::StationError::CdError(cd_error) => {
                                    error_state = ErrorState::CdError;
                                    let cd_error_string = match cd_error {
                                        rradio_messages::CdError::NoCd => "No CD".to_string(),
                                        rradio_messages::CdError::NoCdInfo => {
                                            "No CD information".to_string()
                                        }
                                        rradio_messages::CdError::CdTrayIsOpen => {
                                            "CD tray open".to_string()
                                        }
                                        rradio_messages::CdError::CdTrayIsNotReady => {
                                            "CD tray not ready".to_string()
                                        }
                                        rradio_messages::CdError::FailedToOpenDevice {
                                            code,
                                            message: _,
                                        } => match code {
                                            Some(code_as_int) => match code_as_int {
                                                123 => "CD missing".to_string(),
                                                2 => "No CD drive".to_string(),
                                                _ => {
                                                    format!("got unknown CD error {}", code_as_int)
                                                }
                                            },
                                            None => "CD error but the code was none".to_string(),
                                        },
                                        rradio_messages::CdError::UnknownDiscStatus(size) => {
                                            format!("CD error unknown disk status {}", size)
                                                .to_string()
                                        }
                                        rradio_messages::CdError::UnknownDriveStatus(size) => {
                                            format!("CD error unknown drive status!!{}", size)
                                                .to_string()
                                        }
                                        rradio_messages::CdError::CdIsData1 => {
                                            "This is a data CD so cannot play it. (Data type 1)"
                                                .to_string()
                                        }
                                        rradio_messages::CdError::CdIsData2 => {
                                            "This is a data CD so cannot play it. (Data type 2)"
                                                .to_string()
                                        }
                                        rradio_messages::CdError::CdIsXA21 => {
                                            "This is a data CD so cannot play it. (Data type XA21)"
                                                .to_string()
                                        }
                                        rradio_messages::CdError::CdIsXA22 => {
                                            "This is a data CD so cannot play it. (Data type XA22)"
                                                .to_string()
                                        }
                                        rradio_messages::CdError::CdNotEnabled => {
                                            "CD support is not enabled. You need to recompile"
                                                .to_string()
                                        }
                                        rradio_messages::CdError::IoCtlError { code, message } => {
                                            format!(
                                                "ioctl error {:?} message {}",
                                                code,
                                                message.to_string()
                                            )
                                        }
                                    };
                                    lcd.write_multiline(
                                        lcd::LineNum::Line1,
                                        NUM_CHARACTERS_PER_LINE * 4,
                                        cd_error_string.as_str(),
                                    );
                                    error_message_output = true;
                                }
                                rradio_messages::StationError::BadStationFile(bad_station) => {
                                    error_state = ErrorState::ProgrammerError;
                                    lcd.write_multiline(
                                        lcd::LineNum::Line1,
                                        NUM_CHARACTERS_PER_LINE * 4,
                                        format!("Bad station {}", bad_station.to_string()).as_str(),
                                    );
                                }
                                rradio_messages::StationError::StationsDirectoryIoError {
                                    directory,
                                    err,
                                } => {
                                    lcd.write_multiline(
                                        lcd::LineNum::Line1,
                                        NUM_CHARACTERS_PER_LINE + 4,
                                        format!(
                                            "station error dir={}  error = {}",
                                            directory.to_string(),
                                            err.to_string()
                                        )
                                        .as_str(),
                                    );
                                }
                            }
                        }
                        rradio_messages::CurrentStation::PlayingStation {
                            index,       // this is the channel number in the range 00 to 99
                            source_type, // eg CD
                            title,       // eg Tradcan
                            tracks,
                        } => {
                            /////println!("tracks {:?}", tracks);

                            match index {
                                Some(decoded_index) => {
                                    current_channel = decoded_index.to_string();
                                    artist = "".to_string();
                                    song_title = "".to_string();
                                    album = "".to_string();
                                }
                                None => current_channel = "??".to_string(),
                            }
                            station_type = source_type;
                            line2_text = String::new();
                            number_of_tracks = tracks.as_ref().map_or(0, |tracks| {
                                tracks
                                    .iter() // we iterate through the tracks, excluding those that are merely notifications, & count them
                                    .filter(|track| !track.is_notification)
                                    .count()
                            });

                            let station_type_and_channel = match station_type {
                                rradio_messages::StationType::UPnP => {
                                    format!("Station {}", current_channel)
                                }
                                rradio_messages::StationType::CD => "Playing CD".to_string(),
                                rradio_messages::StationType::Usb => {
                                    format!("USB {}", current_channel)
                                }
                                rradio_messages::StationType::UrlList => {
                                    format!("Station {}", current_channel)
                                }
                            };
                            station_title = title.unwrap_or(ArcStr::new()).to_string();
                            // remove the string "QUIET: " before the start of the station name as it is obvious
                            if station_title.to_lowercase().starts_with("quiet: ") {
                                station_title = station_title["QUIET: ".len()..].to_string();
                            }
                            lcd.write_multiline(
                                lcd::LineNum::Line1,
                                lcd::LINE1_DATA_CHAR_COUNT_USIZE,
                                station_type_and_channel.as_str(),
                            );

                            let message = match station_type {
                                rradio_messages::StationType::CD => {
                                    lcd.write_all_line_2(&format!(
                                        "CD track {} of {}",
                                        current_track_index + 1,
                                        number_of_tracks
                                    ));
                                    "Playing CD ".to_string()
                                }
                                rradio_messages::StationType::Usb => {
                                    format!("USB {}", &current_channel)
                                }
                                rradio_messages::StationType::UrlList
                                | rradio_messages::StationType::UPnP => {
                                    format!("Station {}", &current_channel)
                                } //rradio_messages::StationType::Samba => current_channel.to_string(),
                            };
                            lcd.write_multiline(
                                lcd::LineNum::Line1,
                                LINE1_DATA_CHAR_COUNT_USIZE,
                                message.as_str(),
                            );
                        }
                    }
                }

                if let Some(current_tags) = player_state_difference.current_track_tags {
                    match current_tags {
                        Some(track_tags_decoded) => {
                            if let Some(the_album) = track_tags_decoded.album {
                                album = the_album.to_string();
                            }
                            if let Some(the_artist) = track_tags_decoded.artist {
                                artist = the_artist.to_string();
                            }
                            if let Some(the_title) = track_tags_decoded.title {
                                song_title = the_title.to_string();
                                song_title_scroll_position = 0;
                                lcd.write_with_scroll(
                                    lcd::LineNum::Line3,
                                    lcd::NUM_CHARACTERS_PER_LINE * 2,
                                    song_title.as_ref(),
                                    &mut song_title_scroll_position,
                                );
                            }
                            if let Some(organisation_from_tag) = track_tags_decoded.organisation {
                                organisation = if current_track_index == 0 {
                                    organisation_from_tag.to_string()
                                } else {
                                    format!("{} {}", current_track_index + 1, organisation_from_tag)
                                };
                                if started_up {
                                    lcd.write_all_line_2(&line2_text)
                                }
                            }
                        }
                        _ => {}
                    }

                    //println!("current_tags {:?}", current_tags);
                }

                if let Some(buffering) = player_state_difference.buffering {
                    if song_title.len() <= lcd::NUM_CHARACTERS_PER_LINE
                        && started_up
                        && error_state == ErrorState::NoError
                    {
                        match station_type {
                            rradio_messages::StationType::CD => {} // no need to write the buffer state for CDs
                            _ => lcd.write_buffer_state(buffering),
                        }
                    }
                }
                line2_text = assembleline2(
                    organisation.to_string(),
                    station_title.to_string(),
                    artist.to_string(),
                    album.to_string(),
                    pause_before_playing,
                );
                ////println!(
                ////    "current channel {} organisation {} song_title {}  artist {} line 2 {} ",
                ////    current_channel, organisation, song_title, artist, line2_text
                ////);
            }
        }
    }

    lcd.clear(); // we are ending the program if we get to here
    lcd.write_ascii(lcd::LineNum::Line1, 0, "Ending screen driver");
    lcd.write_multiline(
        lcd::LineNum::Line3,
        lcd::NUM_CHARACTERS_PER_LINE * 2,
        "Computer not shut   down",
    );
    println!("exiting screen driver");

    Ok(())
}

/// Concatontates station_title/organisation, artist & album adding a "/" as a separator. If empty or starts with "unknown", they are skipped.
/// If pause_before_playing > 0 appends " wait" followed by the value of pause_before_playing. Organisation is used if not empty, else station_title is used.
/// Returns the result of the concatonation.
fn assembleline2(
    organisation: String,
    station_title: String,
    mut artist: String,
    mut album: String,
    pause_before_playing: u64,
) -> String {
    let title_shown = {
        if !organisation.is_empty() {
            organisation
        } else {
            station_title.clone()
        }
    };
    if artist.to_lowercase().starts_with("unknown") {
        artist = "".to_string()
    }
    if album.to_lowercase().starts_with("unknown") {
        album = "".to_string()
    }
    let artist_and_album = if artist.is_empty() {
        album
    } else if album.is_empty() {
        artist
    } else {
        format!("{} / {}", artist, album)
    };

    let station_and_artist_and_album = if title_shown.is_empty() {
        artist_and_album
    } else if artist_and_album.is_empty() {
        title_shown
    } else {
        format!("{} / {}", station_title, artist_and_album)
    };
    if pause_before_playing > 0 {
        format!(
            "{} wait{}",
            station_and_artist_and_album, pause_before_playing
        )
    } else {
        station_and_artist_and_album
    }
}
