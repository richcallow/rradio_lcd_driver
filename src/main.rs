use anyhow::Context;
//use chrono::Local;

use futures_util::StreamExt;
//use rradio_messages::{ArcStr, CdError, Event, PipelineState, PlayerStateDiff};
use rradio_messages::{Event, PingTarget, PingTimes, PipelineState, PlayerStateDiff};

mod get_local_ip_address;
mod lcd;

use lcd::{LINE1_DATA_CHAR_COUNT_USIZE, NUM_CHARACTERS_PER_LINE};

mod try_to_kill_earlier_versions_of_lcd_screen_driver;

/// an enum of all the possible error states including those supplied by rradio as enums, gstreamer errors and the error as a string
#[derive(PartialEq, Debug, Clone, Copy)]
pub enum ErrorState {
    NotKnown,
    NoError,
    NoStation,
    CdError,
    CdEjectError,
    MountError,
    UsbOrSambaError,
    GStreamerError,
    ProgrammerError,
    UPnPError,
}
#[derive(PartialEq, Debug, Clone)]
pub struct ErrorList {
    errors_changed: bool,
    error_as_enum: ErrorState,
    error_as_string: String,
}
impl ErrorList {
    pub fn new() -> ErrorList {
        ErrorList {
            errors_changed: false,
            error_as_enum: ErrorState::UPnPError,
            error_as_string: "ddd".to_string(),
        }
    }
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

    let mut zxcurrent_channel_index_as_string = "current_channel_index not initialised".to_string(); //noramlly in the range "00" to "99"; an invalid value, but atleast it is initialised

    let mut zxsource_type = rradio_messages::StationType::UPnP;
    let mut zxstation_title = "no title".to_string();
    let mut zxcurrent_track_index = usize::MAX; // index of whick track on the CD or USB stick
    let mut zxnumber_of_tracks = usize::MAX; // number of tracks on a USB stick or CD or channels on a URL
    let mut zxtrack_title = "no track title".to_string(); // title of a track
    let mut zxorganisation = "no organisation".to_string(); // name of the organisation
    let mut zxartist = "no artist".to_string(); // name of the artist
    let mut zxalbum = "no album".to_string(); // name of the album
    let mut zxis_muted = false; // in practice always false
    let mut zxstored_error_state = ErrorState::NotKnown;

    let mut started_up = false;
    let mut error_state = ErrorState::NotKnown;
    let mut error_state_as_string = String::new();
    let mut pipe_line_state = PipelineState::Null;
    let mut volume = -1_i32;
    //let mut muted = false;
    let mut current_track_index: usize = 0;
    let mut last_current_track_index_to_get_gstreamer_error = String::new(); // an invalid value intentionally;
    let mut current_channel = String::new(); // it is a string
    let mut last_channel_number_not_found = String::new(); // an invalid value intentionally;

    let mut last_channel_number_not_found_not_changed = false;
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
    let mut station_type: rradio_messages::StationType = rradio_messages::StationType::CD; //not true but we have to initiialise it to something
    let mut station_title = String::new();
    let mut station_change_time;
    let mut got_station = false;
    let scroll_period = tokio::time::Duration::from_millis(1600);
    let number_scroll_events_before_scrolling: i32 = 3000 / scroll_period.as_millis() as i32;

    let mut last_scroll_time = tokio::time::Instant::now();
    let mut error_message_output = false;
    let mut pause_before_playing = 0;
    let mut show_temparature_instead_of_gateway_ping = false;
    let mut not_used = false; // the value is never used. it just stops unwanted error messages

    let rradio_events = loop {
        match tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, 8002)).await {
            Ok(stream) => {
                lcd.clear(); //clear out the error message
                lcd.write_ascii(
                    lcd::LineNum::Line1,
                    0,
                    get_local_ip_address::get_local_ip_address().as_str(),
                );
                println!("just output the IP address");
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
                                println!("Bad RRadio Header");
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
                                        "Version mismatch",
                                    );
                                    lcd.write_multiline(
                                        lcd::LineNum::Line2,
                                        lcd::NUM_CHARACTERS_PER_LINE,
                                        &format!("LCD driver: {}", rradio_messages::VERSION),
                                    );
                                    lcd.write_multiline(
                                        lcd::LineNum::Line3,
                                        lcd::NUM_CHARACTERS_PER_LINE,
                                        &format!("rradio:     {}", version.trim_end()),
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
        let timeout_time;
        if error_state == ErrorState::NoStation {
            timeout_time = last_scroll_time + std::time::Duration::from_millis(200);
        }
        // we are not scrolling, but we want to update the time frequently
        else {
            timeout_time = last_scroll_time + scroll_period;
        } // we  are scrolling, so the timeout has to be right for scrolling

        match tokio::time::timeout_at(timeout_time, rradio_events.next()).await {
            Ok(None) => {
                println!("got Ok(None) so exiting");
                break;
            }
            Ok(next_rradio_event_option) => {
                match next_rradio_event_option {
                    Some(next_rradio_event) => {
                        println!("wwwwwwwwwwwwwwww next_rradio_event{:?}", next_rradio_event);
                        match next_rradio_event {
                            Ok(player_state_changed) => {
                                println!("player_state_changed {:?}", player_state_changed);
                                match player_state_changed {
                                    Event::PlayerStateChanged(player_state_difference) => {
                                        {
                                            println!(
                                                "pipeline_state{:?}",
                                                player_state_difference.pipeline_state
                                            );
                                            println!(
                                                "current_station{:?}",
                                                player_state_difference.current_station
                                            );
                                            println!(
                                                "pause_before_playing{:?}",
                                                player_state_difference.pause_before_playing
                                            );
                                            println!(
                                                "current_track_index{:?}",
                                                player_state_difference.current_track_index
                                            );
                                            match player_state_difference.current_track_tags {
                                                Some(current_track_tags) => {
                                                    println!(
                                                        "current_track_tags {:?}",
                                                        current_track_tags
                                                    )
                                                }
                                                None => not_used = true,
                                            }

                                            println!(
                                                "is_muted{:?}",
                                                player_state_difference.is_muted
                                            );
                                            println!("volume{:?}", player_state_difference.volume);
                                            println!(
                                                "buffering{:?}",
                                                player_state_difference.buffering
                                            );
                                            println!(
                                                "track_duration{:?}",
                                                player_state_difference.track_duration
                                            );
                                            println!(
                                                "track_position{:?}",
                                                player_state_difference.track_position
                                            );
                                            println!(
                                                " ping_times{:?}",
                                                player_state_difference.ping_times
                                            );

                                            // find and set the fact we have a gstreamer error
                                            match player_state_difference.latest_error {
                                                Some(latest_error_as_option) => {
                                                    match latest_error_as_option {
                                                        Some(latest_error_temp) => {
                                                            error_state_as_string =
                                                                latest_error_temp.error.to_string();
                                                            error_state =
                                                                ErrorState::GStreamerError;
                                                            zxstored_error_state =
                                                                ErrorState::GStreamerError;
                                                        }
                                                        None => not_used = true,
                                                    }
                                                }
                                                None => {
                                                    println!(
                                                        "latest_error_as_option  said  no error"
                                                    );
                                                    //if error_state != ErrorState::NoStation {
                                                    //error_state = ErrorState::NoError;
                                                    //error_state_as_string = "qqq".to_string();
                                                    //}
                                                }
                                            }
                                        }
                                        {
                                            //find non-gstreamer errors
                                            match player_state_difference.current_station.clone() {
                                                Some(playing_station) => {
                                                    //println!(
                                                    //    "0000000000000000000000000000000  playing_station {:?}",
                                                    //    playing_station
                                                    //);
                                                    match playing_station {
                                                        rradio_messages::CurrentStation::FailedToPlayStation { error } => {
                                                            match error {
                                                                rradio_messages::StationError::StationNotFound {
                                                                    index,
                                                                    directory,
                                                                } => {
                                                                    error_state_as_string = format!("No station {index}");                      // 1 line long
                                                                }
                                                                rradio_messages::StationError::UPnPError(error_string) => {
                                                                    error_state = ErrorState::UPnPError;
                                                                    error_state_as_string = format!("UPnP error {}", error_string.to_string()); //4 lines long
                                                                }
                                                                rradio_messages::StationError::MountError(mount_error) => {
                                                                    error_state = ErrorState::MountError;
                                                                    error_state_as_string = format!("mount error {:?}", mount_error);           // 4 lines long
                                                                }

                                                                rradio_messages::StationError::StationsDirectoryIoError {
                                                                    directory,
                                                                    err,
                                                                } => {
                                                                    error_state = ErrorState::NotKnown;
                                                                    error_state_as_string = format!(                                            //4 lines long
                                                                            "station error dir={}  error = {}", directory, err).to_string();                                                                
                                                                }
                                                                rradio_messages::StationError::BadStationFile(bad_station) => {
                                                                    error_state = ErrorState::ProgrammerError;
                                                                    error_state_as_string =format!("Bad station {}", bad_station);              // 4 lines long
                                                                }
                                                                rradio_messages::StationError::CdError(cd_error) => {
                                                                    error_state = ErrorState::CdError;
                                                                    error_state_as_string = match cd_error {
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
                                                                                123 => "CD missing".to_string(), // windows meaning "The filename, directory name, or volume label syntax is incorrect."
                                                                                2 => "No CD drive".to_string(), // windows meaning "ERROR_FILE_NOT_FOUND"
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
                                                                    }

                                                }
                                            }
                                                        _ => {not_used = true} // we are ignoring all cases escept for errors.
                                                    }
                                                }
                                                None => not_used = true,
                                            };
                                        }

                                        println!("zxcurrent_channel_index_as_string {zxcurrent_channel_index_as_string}     zxsource_type {:?}   zxstation_title {zxstation_title}   zxcurrent_track_index = {zxcurrent_track_index}  zxnumber_of_tracks = {zxnumber_of_tracks}  zxtrack_title {zxtrack_title}   zxorganisation {zxorganisation}  zxartist {zxartist} zxalbum {zxalbum}  error_state {:?}" , zxsource_type , error_state);
                                        println!(
                                            "zxstored_error_state {:?}, error_state_as_string {error_state_as_string}",
                                            zxstored_error_state
                                        );
                                    }
                                }
                            }
                            _ => {
                                println!("none!!!!!!")
                            }
                        }
                    }
                    _ => {
                        println!("match none")
                    }
                }
            }
            Err(_elapsed_message) => {}
        }

        /*
              let next_rradio_event =
                  match tokio::time::timeout_at(timeout_time, rradio_events.next()).await {
                      Ok(None) => break, // No more rradio events, so shutdown
                      Ok(Some(event)) => {
                          println!("<<<<<<<<<<<<<<<<< event {:?}", event);
                          event?
                      }

                      Err(_) => {
                          zxstored_error_state = error_state;
                          println!(
                              "asasasasasasasasasasasasasasasasError state is {:?}",
                              error_state
                          );
                          match zxstored_error_state {
                              ErrorState::NoStation => {
                                  line2_text = String::new();
                                  song_title = String::new();
                                  line2_text_scroll_position = 0;

                                  lcd.write_multiline(
                                      lcd::LineNum::Line2,
                                      lcd::NUM_CHARACTERS_PER_LINE,
                                      get_local_ip_address::get_local_ip_address().as_str(),
                                  );
                                  lcd.write_date_and_time_of_day_line3();
                                  if last_channel_number_not_found_not_changed {
                                      lcd.write_multiline(
                                          lcd::LineNum::Line4,
                                          lcd::NUM_CHARACTERS_PER_LINE,
                                          //"\x00 \x01 \x02 \x03 \x04\x05\x06\x07ñäöü~ÆÇ",
                                          "\x00 \x01 \x02 \x03 \x04\x05\x06\x07ñäöüÆÇç",
                                      );
                                      lcd.write_multiline(
                                          lcd::LineNum::Line1,
                                          lcd::NUM_CHARACTERS_PER_LINE,
                                          compile_time::datetime_str!(),
                                      );
                                      lcd.write_multiline(
                                          lcd::LineNum::Line2,
                                          lcd::NUM_CHARACTERS_PER_LINE,
                                          format!("LCD Version {}", rradio_messages::VERSION).as_str(),
                                      );
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
                                  "Error state: unknown",
                              ),
                              ErrorState::CdError => {
                                  if !error_message_output {
                                      lcd.write_multiline(
                                          lcd::LineNum::Line1,
                                          lcd::NUM_CHARACTERS_PER_LINE * 4,
                                          "Error state: CD error",
                                      )
                                  }
                              }
                              ErrorState::UsbOrSambaError => lcd.write_multiline(
                                  lcd::LineNum::Line1,
                                  lcd::NUM_CHARACTERS_PER_LINE * 4,
                                  "Error state: USB or server error",
                              ),
                              ErrorState::CdEjectError => {} // do nothing as a longer & clearer message already written to all 4 lines
                              ErrorState::UPnPError => {} // do nothing as a longer & clearer message already written to all 4 lines
                              ErrorState::MountError => {} // do nothing as a longer & clearer message already written to all 4 lines
                              ErrorState::ProgrammerError => {} // do nothing as a longer & clearer message already written to all 4 lines
                              ErrorState::GStreamerError => {} // do nothing as a longer & clearer message already written to all 4 lines
                                                               //_ => println!("got unexpected error state {:?}", error_state),
                          }
                          last_scroll_time = timeout_time;

                          println!("about to continueeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");
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

              println!(
                  ">>>>>>>>>>>>>>>>>>>>>next_rradio_event {:?}",
                  next_rradio_event
              );

              match next_rradio_event {
                  Event::PlayerStateChanged(player_state_difference) => {
                      println!(
                          "\r mmmmmmmmmtitle {} organisation {} player_state_differance= {:?}",
                          song_title, organisation, player_state_difference
                      );

                      /*
                                      if let Some(lastest_error_as_option) = player_state_difference.latest_error {
                                          println!("\rlatesterror as option = {:?}", lastest_error_as_option);
                                          match lastest_error_as_option {
                                              Some(latest_error) => {
                                                  error_state_as_string = latest_error.error.to_string();
                                                  error_state = ErrorState::GStreamerError;

                                                  println!(
                                                      "zzzzzzzzzstoring current_channel {:?}",
                                                      current_channel.clone()
                                                  );
                                                  last_current_track_index_to_get_gstreamer_error =
                                                      current_channel.clone();

                                                  println!("got gstreamer error {:?} ", latest_error);

                                                  lcd.write_multiline(
                                                      lcd::LineNum::Line1,
                                                      NUM_CHARACTERS_PER_LINE * 4,
                                                      format!("\rlatest_error{}", error_state_as_string).as_str(),
                                                  )
                                              }
                                              None => {
                                                  error_state_as_string = "rrrrrrrrr".to_string();
                                                  println!("Latest error was none!");
                                              }
                                          }
                                          println!("EEREorstate  = {:?}", error_state);
                                      } else {
                                          error_state = ErrorState::NoError;
                                          ////////////error_state_as_string = "qqq".to_string();
                                      }
                      */

                      //if error_state == ErrorState::NoError
                      //    && last_current_track_index_to_get_gstreamer_error != current_channel
                      //{
                      //    error_state_as_string = "KKKKKKKKKKKKK".to_string();
                      //}

                      ////println!("\r\naaaaaa error_state {:?}", error_state);
                      /////println!("iiiiiiiiiiiiigot gstreamer error {:?} ", error_state_as_string);
                      ////println!("wwwwwcurrent_channel {}", current_channel);
                      ///// println!("vvvvvvvvvvvvv  last_current_track_index_to_get_gstreamer_error {},   latest error {}", last_current_track_index_to_get_gstreamer_error, error_state_as_string);

                      if let Some(pipeline_state) = player_state_difference.pipeline_state {
                          println!("pipeline state is {}", pipeline_state);
                          pipe_line_state = pipeline_state;
                          //if let ErrorState::NoError = error_state {
                          //    lcd.write_volume(pipe_line_state, zxis_muted, volume)
                          //}
                      }
                      //if let Some(is_muted_temp) = player_state_difference.is_muted {
                      //    zxis_muted = is_muted_temp;
                      //    lcd.write_volume(pipe_line_state, zxis_muted, volume);
                      //}

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
                                              "RPing: Rx fail"
                                          }
                                          rradio_messages::PingError::DestinationUnreachable => {
                                              "RDest Unreach"
                                          }
                                          rradio_messages::PingError::Timeout => "RPing NoReply",
                                          rradio_messages::PingError::FailedToSendICMP => "RPing Tx fail",
                                          rradio_messages::PingError::Dns => "RPing DNS err",
                                      }
                                  }
                                  .to_string(),
                              };
                              if pipe_line_state == PipelineState::Playing
                                  && ping_message != PING_TIME_NONE
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

                          if got_station {
                              lcd.write_all_line_2(&format!(
                                  "CD track {} of {}",
                                  current_track_index + 1,
                                  number_of_tracks
                              ));
                          }
                      };
                      //println!("current_track_index {}", current_track_index);

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

                              lcd.write_date_and_time_of_day_line3();
                              lcd.write_temperature_and_time_to_line4();
                          }
                      }

                      if let Some(volume_in) = player_state_difference.volume {
                          volume = volume_in;
                          lcd.write_volume(pipe_line_state, zxis_muted, volume);
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

                      match player_state_difference.current_track_index.clone() {
                          Some(current_track_index_temp) => {
                              zxcurrent_track_index = current_track_index_temp
                          }
                          None => {} //println!("current_track_index_temp is unchanged")}
                      }

                      match player_state_difference.is_muted {
                          // in practice it is never muted; but this will future proof it
                          Some(zxnew_mute_state) => {
                              zxis_muted = zxnew_mute_state;
                          }
                          None => {}
                      }
                      println!("999999999999999999999999999999999  player_state_difference.current_station.clone {:?}", player_state_difference.current_station.clone());

                      match player_state_difference.current_station.clone() {
                          Some(playing_station) => {
                              println!("ggggggggggggggggg playing_station {:?}", playing_station);
                              match playing_station {
                                  rradio_messages::CurrentStation::NoStation => {}
                                  rradio_messages::CurrentStation::FailedToPlayStation { error } => {
                                      println!(
                                          "uuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuuerror is {:?}",
                                          error
                                      );
                                      match error {
                                          rradio_messages::StationError::StationNotFound {
                                              index,
                                              directory: _,
                                          } => {
                                              error_state = ErrorState::NoStation;
                                              current_channel = index.to_string();
                                              song_title = String::new();
                                              line2_text = String::new();
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

                                              last_channel_number_not_found_not_changed =
                                                  last_channel_number_not_found == current_channel;
                                              last_channel_number_not_found = current_channel.clone();
                                              //continue;
                                          }
                                          rradio_messages::StationError::UPnPError(error_string) => {
                                              error_state = ErrorState::UPnPError;
                                              lcd.write_multiline(
                                                  lcd::LineNum::Line1,
                                                  lcd::NUM_CHARACTERS_PER_LINE * 4,
                                                  format!("UPnP error {}", error_string.to_string())
                                                      .as_str(),
                                              )
                                          }
                                          rradio_messages::StationError::MountError(mount_error) => {
                                              error_state = ErrorState::MountError;
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
                                                          123 => "CD missing".to_string(), // windows meaning "The filename, directory name, or volume label syntax is incorrect."
                                                          2 => "No CD drive".to_string(), // windows meaning "ERROR_FILE_NOT_FOUND"
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
                                              zxstored_error_state = error_state;
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
                                                  format!("Bad station {}", bad_station.to_string())
                                                      .as_str(),
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
                                      /*
                                                                      match error_state {
                                                                          ErrorState::NoStation => {
                                                                              zxstored_error_state = ErrorState::NoStation
                                                                          }
                                                                          ErrorState::CdEjectError => {
                                                                              zxstored_error_state = ErrorState::CdEjectError
                                                                          }
                                                                          ErrorState::CdError => {
                                                                              zxstored_error_state = ErrorState::CdError
                                                                          }
                                                                          ErrorState::UPnPError => {
                                                                              zxstored_error_state = ErrorState::UPnPError
                                                                          }
                                                                          ErrorState::GStreamerError => {
                                                                              zxstored_error_state = ErrorState::GStreamerError
                                                                          }
                                                                          ErrorState::MountError => {
                                                                              zxstored_error_state = ErrorState::MountError
                                                                          }
                                                                          ErrorState::NoError => {} // do not reset here wait for an event that clears it instead
                                                                          ErrorState::NotKnown => {
                                                                              zxstored_error_state = ErrorState::NotKnown
                                                                          }
                                                                          ErrorState::ProgrammerError => {
                                                                              zxstored_error_state = ErrorState::ProgrammerError
                                                                          }
                                                                          ErrorState::UsbOrSambaError => {
                                                                              zxstored_error_state = ErrorState::UsbOrSambaError
                                                                          }
                                                                      }
                                      */
                                      println!("zxcurrent_channel_index_as_string {zxcurrent_channel_index_as_string}");
                                  }
                                  rradio_messages::CurrentStation::PlayingStation {
                                      index,       // this is the channel number in the range 00 to 99
                                      source_type, // eg CD
                                      title,       // title of the station eg Tradcan
                                      tracks,      // an array of all the tracks
                                  } => {
                                      //if zxcurrent_track_index >= zxnumber_of_tracks
                                      //{
                                      //    println!("Error current_track_index is {zxcurrent_track_index} and zxnumber_of_tracks is {zxnumber_of_tracks}");
                                      //    zxcurrent_track_index = zxnumber_of_tracks -1;
                                      //}

                                      match index {
                                          Some(new_index) => {
                                              zxcurrent_channel_index_as_string = new_index.to_string();
                                              zxstation_title = String::new();
                                              zxnumber_of_tracks = tracks.as_ref().map_or(0, |tracks| {
                                                  tracks
                                                      .iter() // we iterate through the tracks, excluding those that are merely notifications, & count them
                                                      .filter(|track| !track.is_notification) // a "notification" is a a sound, eg a ding, to say something has occurred as different from something to listen to
                                                      .count()
                                              });
                                              zxtrack_title = String::new(); // title of a track
                                              zxorganisation = String::new(); // name of the organisation
                                              zxartist = String::new(); // name of the artist
                                              zxalbum = String::new(); // name of the album
                                              error_state_as_string = String::new();

                                              println!(
                                                  "about to clear error state it was {:?}",
                                                  error_state
                                              );
                                              error_state = ErrorState::NoError;
                                          }
                                          None => {
                                              zxcurrent_channel_index_as_string = String::new(); // somebody has slected a podcast, so there is on channel number
                                              zxtrack_title = String::new(); // title of a track
                                              zxorganisation = String::new(); // name of the organisation
                                              zxartist = String::new(); // name of the artist
                                              zxalbum = String::new(); // name of the album
                                          }
                                      }

                                      zxsource_type = source_type;
                                      match title.clone() {
                                          Some(new_station_title) => {
                                              zxstation_title = new_station_title.to_string()
                                          }
                                          None => {
                                              println!("station title is unchanged)")
                                          }
                                      }

                                      /*
                                      match tracks.clone()
                                      {
                                          Some (new_tracks_array) => {            // new_tracks_array lists all the tracks in an array
                                              println!("current track is {:?}", new_tracks_array[zxcurrent_track_index]);
                                              //println!("new_tracks_array {:?}",new_tracks_array )


                                              /*let m = new_tracks_array[zxcurrent_track_index];

                                              match m.title {
                                                  Some (track_title_temp) => {println!("track_title_temp {track_title_temp}")},
                                                  None => {}
                                              };*/


                                           },
                                          None => {println!("no new tracks")}
                                      }
                                      */
                                  }
                              }
                          }
                          None => {
                              //println!("pppp player_state_difference.current_station has not changed")
                          }
                      }

                      println!("33333333333333333333333333  player_state_difference.current_station.clone {:?}", player_state_difference.current_station.clone());

                      match player_state_difference.current_track_tags.clone() {
                          Some(current_track_tags_temp) => match current_track_tags_temp {
                              Some(current_track_tags) => {
                                  match current_track_tags.title {
                                      Some(track_title_temp) => {
                                          zxtrack_title = track_title_temp.to_string()
                                      }
                                      None => {}
                                  }
                                  match current_track_tags.organisation {
                                      Some(organisation_temp) => {
                                          zxorganisation = organisation_temp.to_string()
                                      }
                                      None => {}
                                  }
                                  match current_track_tags.artist {
                                      Some(artist_temp) => zxartist = artist_temp.to_string(),
                                      None => {}
                                  }
                                  match current_track_tags.album {
                                      Some(album_temp) => zxalbum = album_temp.to_string(),
                                      None => {}
                                  }
                              }
                              None => {}
                          },
                          None => {}
                      }




                      println!("zxcurrent_channel_index_as_string {zxcurrent_channel_index_as_string}     zxsource_type {:?}   zxstation_title {zxstation_title}   zxcurrent_track_index = {zxcurrent_track_index}  zxnumber_of_tracks = {zxnumber_of_tracks}  zxtrack_title {zxtrack_title}   zxorganisation {zxorganisation}  zxartist {zxartist} zxalbum {zxalbum}  error_state {:?}" , zxsource_type , error_state);
                      println!(
                          "zxstored_error_state {:?}, error_state_as_string {error_state_as_string}",
                          zxstored_error_state
                      );

                      if let Some(current_station) = player_state_difference.current_station {
                          if started_up {
                              got_station = true;
                              lcd.clear();
                          }
                          lcd.write_volume(pipe_line_state, zxis_muted, volume);
                          station_change_time = tokio::time::Instant::now();
                          ////////error_state = ErrorState::NoError;
                          num_of_scrolls_received = 0;
                          line2_text_scroll_position = 0;
                          song_title_scroll_position = 0;
                          song_title = "".to_string();
                          // do NOT set current_track_index to zero here. It is seemingly set whenever the station is changed.
                          /////////////////println!("yyyyyyy current_station {:?}", current_station);

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
                                          line2_text = String::new();
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

                                          last_channel_number_not_found_not_changed =
                                              last_channel_number_not_found == current_channel;
                                          last_channel_number_not_found = current_channel.clone();
                                          //continue;
                                      }
                                      rradio_messages::StationError::UPnPError(error_string) => {
                                          error_state = ErrorState::UPnPError;
                                          lcd.write_multiline(
                                              lcd::LineNum::Line1,
                                              lcd::NUM_CHARACTERS_PER_LINE * 4,
                                              format!("UPnP error {}", error_string.to_string()).as_str(),
                                          )
                                      }
                                      rradio_messages::StationError::MountError(mount_error) => {
                                          error_state = ErrorState::MountError;
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
                                                      123 => "CD missing".to_string(), // windows meaning "The filename, directory name, or volume label syntax is incorrect."
                                                      2 => "No CD drive".to_string(), // windows meaning "ERROR_FILE_NOT_FOUND"
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
                                          if current_channel
                                              != last_current_track_index_to_get_gstreamer_error
                                          {
                                              println!("dddddddddddddderror_state {:?}", error_state);
                                          }
                                          organisation = String::new();
                                          artist = String::new();
                                          song_title = String::new();
                                          station_title = String::new();
                                          album = String::new();
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
                                      rradio_messages::StationType::SambaShare => {
                                          format!("File {}", current_channel)
                                      }
                                  };

                                  if let Some(ye_title) = title {
                                      station_title = ye_title.to_string()
                                  }

                                  //station_title = title.unwrap_or(ArcStr::new()).to_string();
                                  // remove the string "QUIET: " before the start of the station name as it is obvious
                                  if station_title
                                      .to_string()
                                      .to_lowercase()
                                      .starts_with("quiet: ")
                                  {
                                      station_title = station_title["QUIET: ".len()..].to_string();
                                  }
                                  if !station_title.is_empty() {
                                      lcd.write_all_line_2(&station_title)
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
                                      }
                                      rradio_messages::StationType::SambaShare => {
                                          current_channel.to_string()
                                      }
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
                                      line2_text = assembleline2(
                                          organisation.to_string(),
                                          station_title.to_string(),
                                          artist.to_string(),
                                          album.to_string(),
                                          pause_before_playing,
                                      );
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
                      /////println!(
                      //////    "\rorganisation {} station_title {} song_title {}  artist {} album {}  line 2 {} ",
                      /////    organisation, station_title,  song_title, artist, album, line2_text
                      /////);
                  }
              }
        */
    }

    lcd.clear(); // we are ending the program if we get to here
    lcd.write_ascii(lcd::LineNum::Line1, 0, "Ending screen driver");
    lcd.write_multiline(
        lcd::LineNum::Line3,
        lcd::NUM_CHARACTERS_PER_LINE * 2,
        "Computer not shut   down",
    );
    if not_used {
        println!("dummy output")
    };
    println!("exiting screen driver");
    Ok(())
}

/// Concatonates station_title/organisation, artist & album adding a "/" as a separator. If empty or starts with "unknown", they are skipped.
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

/*pub fn get_new_error(pp: PlayerStateDiff) -> ErrorList {
    let mut ret: ErrorList = ErrorList::new();

    ret.errors_changed = true;
    ret.error_as_enum = ErrorState::MountError;
    ret.error_as_string = "333".to_string();
    ret
}
*/
