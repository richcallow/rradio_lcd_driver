use anyhow::Context;
use chrono::Local;

use futures_util::StreamExt;
use rradio_messages::{ArcStr, Event, PipelineState};

mod get_local_ip_address;
mod lcd_screen;
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
    try_to_kill_earlier_versions_of_lcd_screen_driver::try_to_kill_earlier_versions_of_lcd_screen_driver();
    let mut no_connection_counter = 0;

    let mut lcd = lcd_screen::LcdScreen::new()
        .context("Failed to initialize LCD screen")
        .map_err(|err| {
            // Kill other screen drivers here & ourself too
            let name_of_this_executable = std::env::current_exe()
                .expect("Can't get the exec path")
                .to_string_lossy()
                .into_owned();
            println!("in LCD screen: Process was already running so killing all processes called {} including ourself", name_of_this_executable);
            std::process::Command::new("killall").arg(name_of_this_executable).spawn().expect("Failed to kill process");
            err
        })?;

    pretty_env_logger::init(); // options are error, warn, info, debug or trace eg RUST_LOG=info cargo run or RUST_LOG=rradio_lcd_driver=info cargo run

    lcd.write_ascii(
        lcd_screen::LCDLineNumbers::Line1,
        0,
        get_local_ip_address::get_local_ip_address().as_str(),
    );

    println!(
        "Expecting version {} of rradio messages",
        rradio_messages::VERSION
    );

    lcd.write_multiline(
        lcd_screen::LCDLineNumbers::Line2,
        lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 2,
        format!("Expecting version   {} of rradio", rradio_messages::VERSION).as_str(), // the spaces are intentional
    );

    let mut started_up = false;
    let mut error_state = ErrorState::NotKnown;
    let mut pipe_line_state = PipelineState::VoidPending;
    let mut volume = -1_i32;
    let mut current_track_index: usize = 0;
    let mut current_channel;
    let mut line2_text = String::new();
    let mut duration: Option<std::time::Duration> = None;
    let mut number_of_tracks: usize = 0;
    let mut song_title = ArcStr::new();
    let mut num_of_scrolls_received: i32 = 0;
    let mut line2_text_scroll_position: usize = 0;
    let mut song_title_scroll_position: usize = 0;
    let mut artist = ArcStr::new();
    let mut album = ArcStr::new();
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
                lcd.write_multiline(
                    //clear out the error message
                    lcd_screen::LCDLineNumbers::Line2,
                    lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 2,
                    "",
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
                                lcd_screen::LCDLineNumbers::Line1,
                                lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 4,
                                err.to_string().as_str(),
                            );
                        }
                        rradio_messages::BadRRadioHeader::HeaderMismatch {
                            expected: _,
                            actual,
                        } => match std::str::from_utf8(&actual[..]) {
                            // the convert to UTF8 assumes that the characters are ASCII
                            Err(_) => {
                                lcd.write_line(
                                    // oops they were not ASCII so report the problem
                                    lcd_screen::LCDLineNumbers::Line1,
                                    lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE,
                                    "Bad RRadio Header",
                                );
                                lcd.write_line(
                                    lcd_screen::LCDLineNumbers::Line2,
                                    lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE,
                                    "Not UTF-8",
                                );
                                lcd.write_multiline(
                                    lcd_screen::LCDLineNumbers::Line3,
                                    lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 2,
                                    &rradio_messages::DisplayApiHeader(&actual[..]).to_string(),
                                );
                            }
                            Ok(actual_str) => match actual_str.strip_prefix("rradio-messages_") {
                                // it was ASCII; it should start with the string "rradio-messages_"
                                // so try & remove the string, which of course could fail
                                None => {
                                    // it did fail, so it could not have been a rradio message
                                    lcd.write_line(
                                        lcd_screen::LCDLineNumbers::Line1,
                                        lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE,
                                        "Not RRadio",
                                    );
                                    lcd.write_multiline(
                                        lcd_screen::LCDLineNumbers::Line2,
                                        lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 3,
                                        &rradio_messages::DisplayApiHeader(&actual[..]).to_string(),
                                    );
                                }
                                Some(version) => {
                                    // the message was from rradio, but the version was wrong.
                                    lcd.write_line(
                                        lcd_screen::LCDLineNumbers::Line1,
                                        lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE,
                                        "Version Mismatch",
                                    );
                                    lcd.write_line(
                                        lcd_screen::LCDLineNumbers::Line2,
                                        lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE,
                                        &format!("Local:  {}", rradio_messages::VERSION),
                                    );
                                    lcd.write_line(
                                        lcd_screen::LCDLineNumbers::Line3,
                                        lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE,
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
                lcd.write_temperature_and_strength(lcd_screen::LCDLineNumbers::Line4);
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
                    match error_state {
                        ErrorState::NoStation => {
                            lcd.write_temperature_and_strength(lcd_screen::LCDLineNumbers::Line4);
                            lcd.write_date_and_time_of_day_line3();
                            lcd.write_ascii(
                                lcd_screen::LCDLineNumbers::Line2,
                                0,
                                get_local_ip_address::get_local_ip_address().as_str(),
                            );
                        }
                        ErrorState::NoError => {
                            error_message_output = false; // as there is no error, we have not output one
                            if num_of_scrolls_received >= number_scroll_events_before_scrolling {
                                if song_title.len()
                                    > lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 2
                                {
                                    lcd.write_with_scroll(
                                        lcd_screen::LCDLineNumbers::Line3,
                                        lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 2,
                                        song_title.as_ref(),
                                        &mut song_title_scroll_position,
                                    );
                                } else if song_title.len() == 0 && started_up {
                                    // we have space to write the temperature
                                    lcd.write_temperature_and_strength(
                                        lcd_screen::LCDLineNumbers::Line3,
                                    )
                                }
                                if line2_text.len()
                                    > lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE
                                {
                                    lcd.write_with_scroll(
                                        lcd_screen::LCDLineNumbers::Line2,
                                        lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE,
                                        line2_text.as_str(),
                                        &mut line2_text_scroll_position,
                                    );
                                }
                            } else {
                                num_of_scrolls_received += 1; // no need to increment once we have reached the limit & this way we cannot overflow
                            }
                        }
                        ErrorState::NotKnown => println!("Error state: unknown.\n"),
                        ErrorState::CdError => println!("Error state: CD Error\n"),
                        ErrorState::CdEjectError => println!("Error state CD eject error\n"),
                        ErrorState::UPnPError => println!("UPnP error\n"),
                        ErrorState::UsbOrSambaError => {
                            println!("Error state: USB or server Error\n")
                        }
                        ErrorState::ProgrammerError => println!("Error state:: Programmer error\n"),
                        ErrorState::GStreamerError => println!("Error state:: Gstreamer error\n"),
                        //_ => println!("got unexpected error state {:?}", error_state),
                    }
                    last_scroll_time = timeout_time;
                    continue;
                }
            };

        log::info!("Event: {:?}", next_rradio_event);

        //println!("Event: {:?}", next_rradio_event);

        if !started_up {
            if let Event::PlayerStateChanged(rradio_messages::PlayerStateDiff {
                current_station: Some(Some(_)),
                ..
            }) = &next_rradio_event
            {
                started_up = true;
            }
        }

        match next_rradio_event {
            Event::LogMessage(log_message) => {
                println!("Error {:?}", log_message);
                match log_message {
                    rradio_messages::LogMessage::Error(error_message) => {
                        let displayed_error_message = match error_message {
                            rradio_messages::Error::NoPlaylist => "No playlist found".to_string(),

                            rradio_messages::Error::InvalidTrackIndex(index) => {
                                error_state = ErrorState::ProgrammerError;
                                format!("Programmer Error could not find track index{}", index)
                            }
                            rradio_messages::Error::PipelineError(error) => {
                                error_state = ErrorState::GStreamerError;
                                //println!("Gstreamer error{}", error.0);
                                format!("             GStreamer Error {}", error.0)
                                //13 spaces so it is not overwrtten by the ping time
                            }
                            rradio_messages::Error::StationError(
                                rradio_messages::StationError::CdError(cderr),
                            ) => {
                                error_state = ErrorState::CdError;
                                println!("CD ERRRR {}", cderr);
                                match cderr {
                                    rradio_messages::CdError::FailedToOpenDevice(error_string) => {
                                        let os_error = String::from("os error ");
                                        let pos = error_string.find(os_error.as_str());
                                        if let Some(mut position) = pos {
                                            position += os_error.len();
                                            let error_string_shortened = &error_string[position..];

                                            match error_string_shortened {
                                                "123)" => format!("CD missing"),
                                                "2)" => format!("No CD drive"),
                                                _ => {
                                                    println!(
                                                        "got unknown CD error {}",
                                                        error_string_shortened
                                                    );
                                                    format!(
                                                        "got unknown CD error {}",
                                                        error_string_shortened
                                                    )
                                                }
                                            }
                                        } else {
                                            "Could not identify the CD error".to_string()
                                        }
                                    }
                                    rradio_messages::CdError::CdNotEnabled => {
                                        error_message_output = false; //always want this message output
                                        "CD support is not enabled. You need to recompile"
                                            .to_string()
                                    }
                                    rradio_messages::CdError::IoCtlError(s) => {
                                        error_message_output = false; //always want this message output
                                        format!("CD IOCTL error {:?}", s)
                                    }
                                    rradio_messages::CdError::NoCdInfo => {
                                        error_message_output = false; //always want this message output
                                        "No CD info".to_string()
                                    }
                                    rradio_messages::CdError::NoCd => "No CD found".to_string(),
                                    rradio_messages::CdError::CdTrayIsNotReady => {
                                        error_message_output = false; //always want this message output
                                        "CD tray is not ready".to_string()
                                    }
                                    rradio_messages::CdError::CdTrayIsOpen => {
                                        error_message_output = false; //always want this message output
                                        "CD tray is open".to_string()
                                    }
                                    rradio_messages::CdError::CdIsData1 => {
                                        error_message_output = false; //always want this message output
                                        "This is a data CD so cannot play it. (Data type 1)"
                                            .to_string()
                                    }
                                    rradio_messages::CdError::CdIsData2 => {
                                        error_message_output = false; //always want this message output
                                        "This is a data CD so cannot play it. (Data type 2)"
                                            .to_string()
                                    }
                                    rradio_messages::CdError::CdIsXA21 => {
                                        error_message_output = false; //always want this message output
                                        "This is a data CD so cannot play it. (Data type XA21)"
                                            .to_string()
                                    }
                                    rradio_messages::CdError::CdIsXA22 => {
                                        error_message_output = false; //always want this message output

                                        "This is a data CD so cannot play it. (Data type XA22"
                                            .to_string()
                                    }
                                    rradio_messages::CdError::UnknownDriveStatus(size) => {
                                        error_message_output = false; //always want this message output
                                        format!("CD error unknown drive status {}", size)
                                    }
                                    rradio_messages::CdError::UnknownDiscStatus(size) => {
                                        error_message_output = false; //always want this message output
                                        format!("CD error unknown disk status {}", size)
                                    }
                                }
                            }
                            rradio_messages::Error::StationError(
                                rradio_messages::StationError::MountError(mount_err),
                            ) => {
                                error_state = ErrorState::UsbOrSambaError;
                                match mount_err {
                                    rradio_messages::MountError::UsbNotEnabled => {
                                        "USB not enabled. You need to recompile".to_string()
                                    }
                                    rradio_messages::MountError::SambaNotEnabled => {
                                        "You must recompile with file server enabled".to_string()
                                    }
                                    rradio_messages::MountError::CouldNotCreateTemporaryDirectory(
                                        dir,
                                    ) => {
                                        error_message_output = false;           //always want this message output
                                        format!("Mount (USB or file server) error: Could not create temporary directory {} ", dir)},
                                    rradio_messages::MountError::CouldNotMountDevice {
                                        device,
                                        ..
                                    } => {
                                        error_message_output = false;           //always want this message output
                                        format!("Could not mount device {} ", device)},
                                    rradio_messages::MountError::NotFound => {
                                        error_message_output = false;           //always want this message output
                                        "Device not found".to_string()
                                    }

                                    rradio_messages::MountError::ErrorFindingTracks(s) => {
                                        error_message_output = false;           //always want this message output
                                        format!("Mount (USB or file server) : Error finding tracks {}", s)
                                    }
                                    rradio_messages::MountError::TracksNotFound => {
                                        "Mount (USB or file server) error: Tracks not found".to_string()
                                    }
                                }
                            }
                            rradio_messages::Error::StationError(
                                rradio_messages::StationError::UPnPError(err),
                            ) => {
                                error_message_output = false; //always want this message output
                                error_state = ErrorState::UPnPError;
                                format!("UPnP error {}", err)
                            }

                            rradio_messages::Error::StationError(
                                rradio_messages::StationError::StationsDirectoryIoError {
                                    directory,
                                    err,
                                },
                            ) => {
                                error_message_output = false; //always want this message output
                                error_state = ErrorState::ProgrammerError;
                                format!(
                                    "Station directory IO error {} in directory {}",
                                    err, directory
                                )
                            }

                            rradio_messages::Error::StationError(
                                rradio_messages::StationError::BadStationFile(err),
                            ) => {
                                error_state = ErrorState::ProgrammerError;
                                format!("Bad station file {}", err)
                            }
                            rradio_messages::Error::StationError(
                                rradio_messages::StationError::StationNotFound { index, .. },
                            ) => {
                                error_state = ErrorState::NoStation;
                                current_channel = index.to_string();
                                song_title = ArcStr::new();
                                line2_text = "".to_string();
                                current_track_index = 0;
                                station_title = String::new();
                                number_of_tracks = 0;
                                duration = None;
                                num_of_scrolls_received = 0;
                                line2_text_scroll_position = 0;
                                song_title_scroll_position = 0;
                                lcd.clear();
                                lcd.write_line(
                                    lcd_screen::LCDLineNumbers::Line1,
                                    lcd_screen::LCDLineNumbers::LINE1_DATA_CHAR_COUNT,
                                    format!("No station {}", current_channel).as_str(),
                                );
                                lcd.write_temperature_and_strength(
                                    lcd_screen::LCDLineNumbers::Line4,
                                );
                                lcd.write_ascii(
                                    lcd_screen::LCDLineNumbers::Line4,
                                    17,
                                    Local::now().format("%a").to_string().as_str(),
                                );
                                lcd.write_date_and_time_of_day_line3();
                                continue;
                            }
                            rradio_messages::Error::TagError(tag_error) => {
                                format!("Got a tag error {}", tag_error)
                            }
                            rradio_messages::Error::EjectError(_) => {
                                //error_message_output = true;
                                "CD EjectError".to_string()
                            } /*_ => {
                                  error_state = ErrorState::NotKnown;
                                  lcd.write_all_line_2("Got unhandled error");
                                  continue;
                              }*/
                        };
                        if !error_message_output {
                            lcd.write_multiline(
                                lcd_screen::LCDLineNumbers::Line1,
                                lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 4,
                                &displayed_error_message,
                            );
                            error_message_output = true; //we want to output the first error message only to the screen
                        } // but all the messages to the HDMI screen as the latter can display the entire history
                        println!("got error message {}", displayed_error_message.to_string());
                    }
                }
            }

            Event::PlayerStateChanged(diff) => {
                //println! ("diff {:?}" ,diff);
                use rradio_messages::{PingTarget, PingTimes};
                if started_up && station_change_time.elapsed() > std::time::Duration::from_secs(6) {
                    if let Some(ref ping_times) = diff.ping_times {
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
                                    format!("LocPing{:>3}ms", gateway_ping.as_nanos() / 1000_000)
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
                                    format!("RemPing{:>3}ms", remote_ping.as_nanos() / 1000_000)
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
                        if ping_message != PING_TIME_NONE {
                            lcd.write_line(
                                lcd_screen::LCDLineNumbers::Line1,
                                lcd_screen::LCDLineNumbers::LINE1_DATA_CHAR_COUNT,
                                &ping_message,
                            )
                        };
                    }
                }

                if !started_up {
                    if let Some(ping_times) = diff.ping_times {
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
                            lcd.write_line(
                                lcd_screen::LCDLineNumbers::Line2,
                                lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE,
                                &ping_message,
                            );
                        };

                        if  !started_up{
                            lcd.write_date_and_time_of_day_line3();
                           lcd.write_temperature_and_time_to_line4();
                         }
                    }
                }

                if let Some(current_track_tags) = diff.current_track_tags {
                    //println!("1current track tags {:?}", current_track_tags);

                    if let Some(track_tags) = current_track_tags {
                        /*println!(
                            "2current track_tags{:?}, current_tract_index{}",
                            track_tags, current_track_index
                        );*/
                        if let Some(organisation_from_tag) = track_tags.organisation {
                            line2_text = if current_track_index == 0 {
                                organisation_from_tag.to_string()
                            } else {
                                format!("{} {}", current_track_index + 1, organisation_from_tag)
                            };
                            if started_up {
                                lcd.write_all_line_2(&line2_text)
                            }
                        }
                        if let Some(artist_from_tag) = track_tags.artist {
                            artist = artist_from_tag;
                            line2_text = assembleline2(
                                station_title.to_string(),
                                artist.to_string(),
                                album.to_string(),
                                pause_before_playing,
                            );
                            if line2_text.len() > 0 {
                                lcd.write_all_line_2(&line2_text)
                            }
                        }
                        if let Some(album_from_tag) = track_tags.album {
                            album = album_from_tag;
                            line2_text = assembleline2(
                                station_title.to_string(),
                                artist.to_string(),
                                album.to_string(),
                                pause_before_playing,
                            );
                            if line2_text.len() > 0 {
                                lcd.write_all_line_2(&line2_text)
                            }
                        }
                        song_title = track_tags.title.unwrap_or_default();
                        //println!("ye_tag_title {}", song_title);
                        if started_up {
                            lcd.write_multiline(
                                lcd_screen::LCDLineNumbers::Line3,
                                lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 2,
                                song_title.as_ref(),
                            );
                        }
                    } else {
                        if error_state != ErrorState::GStreamerError {}
                        // the tags have changed to be "" so we must blank them
                        song_title = ArcStr::new();
                        artist = ArcStr::new();
                        album = ArcStr::new();
                        if error_state != ErrorState::GStreamerError {
                            // do not overwrite gstreamer errors
                            lcd.write_multiline(
                                lcd_screen::LCDLineNumbers::Line3,
                                lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 2,
                                "",
                            );
                        }
                    };
                }
                if let Some(current_station) = diff.current_station {
                    duration = None;
                    error_message_output = false; // as there is no error, we have not output one
                    song_title = ArcStr::new();
                    artist = ArcStr::new();
                    album = ArcStr::new();
                    num_of_scrolls_received = 0;
                    line2_text_scroll_position = 0;
                    line2_text = String::new();
                    song_title_scroll_position = 0;
                    current_track_index = 0;
                    if let Some(station) = current_station {
                        if started_up {
                            //println!("clearing screen");
                            got_station = true;
                            error_state = ErrorState::NoError;
                            lcd.clear()
                        };
                        station_type = station.source_type;
                        number_of_tracks = station.tracks.as_ref().map_or(0, |tracks| {
                            tracks
                                .iter() // we iterate through the tracks, excluding those that are merely notifications, & count them
                                .filter(|track| !track.is_notification)
                                .count()
                        });
                        //println!("Current Station{:?} with {} tracks",station, number_of_tracks);
                        match station_type {
                            rradio_messages::StationType::CD => {
                                pause_before_playing = 0;
                            }
                            rradio_messages::StationType::Usb => {
                                pause_before_playing = 0;
                            }
                            _ => {}
                        }
                        station_change_time = tokio::time::Instant::now();
                        station_title = station.title.unwrap_or(ArcStr::new()).to_string();
                        // remove the string "QUIET: " before the start of the station name as it is obvious
                        if station_title.to_lowercase().starts_with("quiet: ") {
                            station_title = station_title["QUIET: ".len()..].to_string();
                        }
                        current_channel =
                            String::from(station.index.as_ref().map_or("", |index| index.as_str()));
                        if let Some(first_track) = station.tracks.as_deref().and_then(|tracks| {
                            tracks.iter().filter(|track| !track.is_notification).next()
                        }) {
                            if let Some(artist_from_track) = &first_track.artist {
                                artist = artist_from_track.clone();
                                line2_text = assembleline2(
                                    station_title.to_string(),
                                    artist.to_string(),
                                    album.to_string(),
                                    pause_before_playing,
                                );
                                lcd.write_all_line_2(&line2_text);
                                song_title_scroll_position = 0;
                            }
                            if let Some(album_from_track) = &first_track.album {
                                album = album_from_track.clone();
                                line2_text = assembleline2(
                                    station_title.to_string(),
                                    artist.to_string(),
                                    album.to_string(),
                                    pause_before_playing,
                                );
                                lcd.write_all_line_2(&line2_text);
                                song_title_scroll_position = 0;
                            }
                            if let Some(title_from_track) = &first_track.title {
                                song_title = title_from_track.clone();
                                lcd.write_multiline(
                                    lcd_screen::LCDLineNumbers::Line3,
                                    lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 2,
                                    song_title.as_str(),
                                );
                            }
                        } // else do nothing as there were no tracks
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
                            }
                            rradio_messages::StationType::Samba => {
                                format!("{}", &current_channel)
                            }
                        };
                        lcd.write_line(
                            lcd_screen::LCDLineNumbers::Line1,
                            lcd_screen::LCDLineNumbers::LINE1_DATA_CHAR_COUNT,
                            message.as_str(),
                        );
                        //println!("current_channel {}", current_channel);
                        let st = assembleline2(
                            station_title.to_string(),
                            artist.to_string(),
                            album.to_string(),
                            pause_before_playing,
                        );

                        line2_text = if current_track_index == 0 {
                            st
                        } else {
                            format!("{} {}", current_track_index + 1, st)
                        };

                        if started_up
                            && line2_text.len() > 0
                            && artist.len() == 0
                            && album.len() == 0
                        {
                            lcd.write_all_line_2(&line2_text)
                        }
                    } else {
                        got_station = false;
                    }
                }
                if let Some(pause_before_playing_in) = diff.pause_before_playing {
                    if let Some(pause_before_playing_as_duration) = pause_before_playing_in {
                        pause_before_playing = pause_before_playing_as_duration.as_secs();
                    } else {
                        pause_before_playing = 0;
                    }
                }
                //println!("pause time = {}", pause_before_playing);
                if let Some(current_track_index_in) = diff.current_track_index {
                    current_track_index = current_track_index_in;
                    if started_up {
                        match station_type {
                            rradio_messages::StationType::CD => {
                                lcd.write_all_line_2(&format!(
                                    "CD track {} of {}.",
                                    current_track_index + 1,
                                    number_of_tracks
                                ));
                            }
                            _ => {} // patterns `UrlList`, `FileServer` and `USB` not covered
                        }
                    }
                    num_of_scrolls_received = 0;
                    line2_text_scroll_position = 0;
                    song_title_scroll_position = 0;
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
                    if song_title.len() <= lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE
                        && started_up
                    {
                        match error_state {
                            ErrorState::NoError => {
                                match station_type {
                                    rradio_messages::StationType::CD => {} // no need to write the buffer state for CDs
                                    _ => lcd.write_buffer_state(buffering),
                                }
                            }
                            _ => {} // here we only want to match the "no error condition"
                        }
                    }
                }
                if let Some(track_duration_in) = diff.track_duration {
                    duration = track_duration_in;
                }
                if let Some(position) = diff.track_position {
                    if let Some((duration, position)) = duration.zip(position) {
                        if got_station && pipe_line_state == rradio_messages::PipelineState::Playing
                        {
                            match error_state {
                                ErrorState::NoError => {
                                    let track_index = current_track_index + 1; // humans count from 1
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
                                        9 => format!(
                                            "{}:{}of {}",
                                            track_index, position_secs, duration_secs
                                        ),
                                        10 => format!(
                                            "{}: {}of{}",
                                            track_index, position_secs, duration_secs
                                        ),
                                        _ => format!("{}: {}", track_index, position_secs),
                                    };
                                    if (position_secs >= 2) | (current_track_index > 0) {
                                        //wait 2 seconds so that people can read what comes before for the first track
                                        lcd.write_line(
                                            lcd_screen::LCDLineNumbers::Line1,
                                            lcd_screen::LCDLineNumbers::LINE1_DATA_CHAR_COUNT,
                                            message.as_str(),
                                        );
                                    }
                                }
                                _ => {} // here we only want to match the "no error condition"
                            }
                        }
                    }
                }
            }
        }
    }

    lcd.clear(); // we are ending the program if we get to here
    lcd.write_ascii(lcd_screen::LCDLineNumbers::Line1, 0, "Ending screen driver");
    lcd.write_multiline(
        lcd_screen::LCDLineNumbers::Line3,
        lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 2,
        "Computer not shut   down",
    );
    println!("exiting screen driver");

    Ok(())
}

fn assembleline2(
    station_title: String,
    mut artist: String,
    mut album: String,
    pause_before_playing: u64,
) -> String {
    if artist.to_lowercase().starts_with("unknown") {
        artist = "".to_string()
    }
    if album.to_lowercase().starts_with("unknown") {
        album = "".to_string()
    }
    let artist_and_title = if artist.len() == 0 {
        album
    } else if album.len() == 0 {
        artist
    } else {
        format!("{} / {}", artist, album)
    };

    let station_and_artist_and_title = if station_title.len() == 0 {
        artist_and_title
    } else if artist_and_title.len() == 0 {
        station_title
    } else {
        format!("{} / {}", station_title, artist_and_title)
    };
    if pause_before_playing > 0 {
        format!(
            "{} wait{}",
            station_and_artist_and_title, pause_before_playing
        )
    } else {
        station_and_artist_and_title
    }
}
