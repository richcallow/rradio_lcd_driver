use anyhow::Context;
use psutil::process::Process;
use rradio_messages::PipelineState;
use smol::io::AsyncReadExt;

mod get_local_ip_address;
mod lcd_screen;

type Event = rradio_messages::Event<String, String, Vec<rradio_messages::Track>>;

#[derive(PartialEq, Debug)]
pub enum ErrorState {
    NotKnown,
    NoError,
    NoStation,
    CdError,
    Usberror,
    GStreamerError,
    ProgrammerError,
}

fn main() -> Result<(), anyhow::Error> {
    try_to_kill_earlier_versions_of_lcd_screen_driver();

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
    lcd.write_temperature_and_time();
    lcd.write_multiline(
        lcd_screen::LCDLineNumbers::Line2,
        lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 2,
        format!("No connection to    internal server").as_str(), // the spaces are intentional
    );

    smol::block_on(async move {
        let mut started_up = false;
        let mut error_state = ErrorState::NotKnown;
        let mut pipe_line_state: PipelineState = PipelineState::VoidPending;
        let mut volume: i32 = -1;
        let mut current_track_index: usize = 0;
        let mut current_channel: String;
        let mut line2_text: String = "".to_string();
        let mut duration: Option<std::time::Duration> = None;
        let mut number_of_tracks: usize = 0;
        let mut song_title = String::new();
        let mut num_of_scrolls_received: i32 = 0;
        let mut line2_text_scroll_position: usize = 0;
        let mut song_title_scroll_position: usize = 0;
        let mut artist: String = "".to_string();
        let mut album: String = "".to_string();
        let mut station_type: rradio_messages::StationType = rradio_messages::StationType::CD;
        let mut got_station = false;
        let scroll_period = std::time::Duration::from_millis(1500);
        let number_scroll_events_before_scrolling: i32 = 4000 / scroll_period.as_millis() as i32;
        let mut last_scroll_time = std::time::Instant::now();

        let mut connection = {
            loop {
                match smol::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, 8002)).await {
                    Ok(c) => {
                        lcd.write_multiline(
                            //clear out the error message
                            lcd_screen::LCDLineNumbers::Line2,
                            lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 2,
                            "",
                        );
                        break c;
                    }
                    Err(error) => {
                        println!("Connection Error: {:?}", error);
                        lcd.write_temperature_and_time();
                        std::thread::sleep(std::time::Duration::from_millis(1000));
                    }
                }
            }
        };

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
                            lcd.write_temperature_line4();
                            lcd.write_date_and_time_of_day_line3();
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
                            if !started_up {
                                lcd.write_temperature_and_time();
                            }
                        }
                        ErrorState::NotKnown => println!("Error state: unknown"),
                        ErrorState::CdError => println!("Error state: CD Error"),
                        ErrorState::Usberror => println!("Error state: USB Error"),
                        ErrorState::ProgrammerError => println!("Error state:: Programmer error"),
                        ErrorState::GStreamerError => println!("Error state:: Gstreamer error"),
                        //_ => println!("got unexpected error state {:?}", error_state),
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

            log::debug!("length {},   {:?}", message_length, buffer);

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
                Event::LogMessage(log_message) => {
                    match log_message {
                        rradio_messages::LogMessage::Error(error_message) => {
                            println!("Error message: {}", error_message);
                            let displayed_error_message = match error_message {
                            rradio_messages::Error::NoPlaylist
                            | rradio_messages::Error::InvalidTrackIndex(..) => {
                                error_state = ErrorState::ProgrammerError;
                                "Programmer Error".to_string()
                            }
                            rradio_messages::Error::PipelineError(..) => {
                                error_state = ErrorState::GStreamerError;
                                "GStreamer Error".to_string()
                            }
                            rradio_messages::Error::StationError(
                                rradio_messages::StationError::CdError(cderr),
                            ) => {
                                error_state = ErrorState::CdError;
                                println!("CD ERRRR {}", cderr);
                                match cderr {
                                    rradio_messages::CdError::CannotOpenDevice(error_string) => {
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
                                            format!("Could not identify the CD error")
                                        }
                                    }
                                    rradio_messages::CdError::CdNotEnabled => {
                                        "CD support is not enabled. You need to recompile".to_string()
                                    }
                                    rradio_messages::CdError::IoCtlError(s) => {
                                        format!("CD IOCTL error {:?}", s)
                                    }
                                    rradio_messages::CdError::NoCdInfo => "No CD info".to_string(),
                                    rradio_messages::CdError::NoCd => "No CD found".to_string(),
                                    rradio_messages::CdError::CdTrayIsNotReady => {
                                        "CD tray is not ready".to_string()
                                    }
                                    rradio_messages::CdError::CdTrayIsOpen => {
                                        "CD tray is open".to_string()
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
                                        "This is a data CD so cannot play it. (Data type XA22"
                                            .to_string()
                                    }
                                    rradio_messages::CdError::UnknownDriveStatus(size) => {
                                        format!("CD error unknown drive status {}", size)
                                    }
                                    rradio_messages::CdError::UnknownDiscStatus(size) => {
                                        format!("CD error unknown disk status {}", size)
                                    }
                                }
                            }
                            rradio_messages::Error::StationError(
                                rradio_messages::StationError::UsbError(usberr),
                            ) => {
                                error_state = ErrorState::Usberror;
                                println!("USB ERRRR {:?}", usberr);
                                match usberr {
                                    rradio_messages::UsbError::UsbNotEnabled => {
                                        "USB not enabled. You need to recompile".to_string()
                                    }
                                    rradio_messages::UsbError::CouldNotCreateTemporaryDirectory(
                                        dir,
                                    ) => format!("USB error: Could not create temporary directory {} ", dir),
                                    rradio_messages::UsbError::CouldNotMountDevice {
                                        device,
                                        ..
                                    } => format!("Could not mount device {} ", device),
                                    rradio_messages::UsbError::UsbNotConnected => {
                                        "No USB device found".to_string()
                                    }

                                    rradio_messages::UsbError::ErrorFindingTracks(s) => {
                                        format!("USB error: Error finding tracks {}", s)
                                    }
                                    rradio_messages::UsbError::TracksNotFound => {
                                        "USB error: Tracks not found".to_string()
                                    }
                                }
                            }
                            .to_string(),
                            rradio_messages::Error::StationError(
                                rradio_messages::StationError::StationNotFound { index, .. },
                            ) => {
                                error_state = ErrorState::NoStation;
                                current_channel = index;
                                song_title = "".to_string();
                                line2_text = "".to_string();
                                current_track_index = 0;
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
                                lcd.write_temperature_line4();
                                lcd.write_date_and_time_of_day_line3();
                                continue;
                            }
                            //*********** TagError not covered */
                            _ => {
                                lcd.write_all_line_2("Got unhandled error");
                                continue;
                            }
                        };
                            println!("got error message {}", displayed_error_message);
                            lcd.write_multiline(
                                lcd_screen::LCDLineNumbers::Line1,
                                lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 4,
                                &displayed_error_message,
                            )
                        }
                    }
                }
                Event::PlayerStateChanged(diff) => {
                    if let Some(current_station) = diff.current_station.into_option() {
                        if started_up {
                            lcd.clear()
                        };
                        duration = None;
                        got_station = true;
                        error_state = ErrorState::NoError;
                        song_title = "".to_string();
                        artist = "".to_string();
                        album = "".to_string();
                        num_of_scrolls_received = 0;
                        line2_text_scroll_position = 0;
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

                            if number_of_tracks > 0 {
                                let first_track = &station.tracks[0];
                                {
                                    if let Some(artist_from_track) = &first_track.artist {
                                        artist = artist_from_track.to_string();
                                        line2_text =
                                            assembleline2(artist.to_string(), album.to_string());
                                        lcd.write_all_line_2(&line2_text);
                                        song_title_scroll_position = 0;
                                    }
                                    if let Some(album_from_track) = &first_track.album {
                                        album = album_from_track.to_string();
                                        line2_text =
                                            assembleline2(artist.to_string(), album.to_string());
                                        lcd.write_all_line_2(&line2_text);
                                        song_title_scroll_position = 0;
                                    }
                                    if let Some(title_from_track) = &first_track.title {
                                        song_title = title_from_track.to_string();
                                        lcd.write_multiline(
                                            lcd_screen::LCDLineNumbers::Line3,
                                            lcd_screen::LCDLineNumbers::NUM_CHARACTERS_PER_LINE * 2,
                                            song_title.as_str(),
                                        );
                                    }
                                };
                            }
                            current_channel = station.index.unwrap_or_else(|| "??".to_string());
                            let message = match station_type {
                                rradio_messages::StationType::CD => {
                                    lcd.write_all_line_2(&format!(
                                        "CD track {} of {}",
                                        current_track_index + 1,
                                        number_of_tracks
                                    ));
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
                            //println!("current_channel {}", current_channel);
                            let st = station.title.unwrap_or_else(|| line2_text);

                            line2_text = if current_track_index == 0 {
                                st
                            } else {
                                format!("{} {}", current_track_index + 1, st)
                            };

                            if started_up && line2_text.len() > 0 {
                                lcd.write_all_line_2(&line2_text)
                            }
                        } else {
                            got_station = false;
                        }
                    }
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
                    if let Some(current_track_tags) = diff.current_track_tags.into_option() {
                        if let Some(track_tags) = current_track_tags {
                            println!(
                                "current track_tags{:?}, current_tract_index{}",
                                track_tags, current_track_index
                            );
                            if let Some(organisation_from_tag) = track_tags.organisation {
                                line2_text = if current_track_index == 0 {
                                    organisation_from_tag
                                } else {
                                    format!("{} {}", current_track_index + 1, organisation_from_tag)
                                };
                                if started_up {
                                    lcd.write_all_line_2(&line2_text)
                                }
                            }
                            if let Some(artist_from_tag) = track_tags.artist {
                                artist = artist_from_tag;
                                line2_text = assembleline2(artist.to_string(), album.to_string());
                                if line2_text.len() > 0 {
                                    lcd.write_all_line_2(&line2_text)
                                }
                            }
                            if let Some(album_from_tag) = track_tags.album {
                                album = album_from_tag;
                                line2_text = assembleline2(artist.to_string(), album.to_string());
                                if line2_text.len() > 0 {
                                    lcd.write_all_line_2(&line2_text)
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
                            line2_text_scroll_position = 0;
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
                    if let Some(track_duration_in) = diff.track_duration.into_option() {
                        duration = track_duration_in;
                    }
                    if let Some(position) = diff.track_position.into_option() {
                        if let Some((duration, position)) = duration.zip(position) {
                            if got_station
                                && pipe_line_state == rradio_messages::PipelineState::Playing
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
                                                "{}, {} of {}",
                                                track_index, position_secs, duration_secs
                                            ),
                                            8 => format!(
                                                "{},{} of {}",
                                                track_index, position_secs, duration_secs
                                            ),
                                            9 => format!(
                                                "{},{}of {}",
                                                track_index, position_secs, duration_secs
                                            ),
                                            10 => format!(
                                                "{}, {}of{}",
                                                track_index, position_secs, duration_secs
                                            ),
                                            _ => format!("{}, {}", track_index, position_secs),
                                        };
                                        lcd.write_line(
                                            lcd_screen::LCDLineNumbers::Line1,
                                            lcd_screen::LCDLineNumbers::LINE1_DATA_CHAR_COUNT,
                                            message.as_str(),
                                        );
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
    })
}
fn assembleline2(mut artist: String, mut album: String) -> String {
    if artist.to_lowercase().starts_with("unknown") {
        artist = "".to_string()
    }
    if album.to_lowercase().starts_with("unknown") {
        album = "".to_string()
    }
    if artist.len() == 0 {
        album
    } else if album.len() == 0 {
        artist
    } else {
        format!("{} / {}", artist, album)
    }
}
fn try_to_kill_earlier_versions_of_lcd_screen_driver() -> () {
    let me = procfs::process::Process::myself().unwrap();
    //println!("my pid is {}", me.pid);

    for proc in procfs::process::all_processes().unwrap()
    // only fails if there is no "/proc" folder
    {
        if proc.stat.comm == me.stat.comm && proc.pid != me.pid {
            println!("got a pid  other than me {}", proc.pid);
            if let Ok(process) = Process::new(proc.pid as u32) {
                if let Err(error) = process.kill() {
                    println!(
                        "Failed to kill another LCD driver program due to error {}.",
                        error
                    );
                }
            } else {
                println!(
                    "Got error when trying to kill process with PID {}",
                    proc.pid
                );
            };
        }; // else it was either not an LCD screen driver or it was us.
    }
}
