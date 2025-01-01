//use procfs::ProcError;
use psutil::process::Process;
pub fn try_to_kill_earlier_versions_of_lcd_screen_driver() {
    let wrapped_me = procfs::process::Process::myself();
    match wrapped_me {
        Ok(me) => {
            match me.stat() {
                Ok(me_stat) => {
                    for proc in procfs::process::all_processes().unwrap()
                    // only fails if there is no "/proc" folder
                    {
                        match proc {
                            Ok(one_process) => match one_process.stat() {
                                Ok(stat) => {
                                    if stat.comm == me_stat.comm && one_process.pid() != me.pid() {
                                        println!("Got a pid  other than me {}", one_process.pid);
                                        if let Ok(process) = Process::new(one_process.pid as u32) {
                                            if let Err(error) = process.kill() {
                                                println!(
                                                    "Failed to kill another LCD driver program due to error {}.",
                                                    error
                                                );
                                            }
                                        } else {
                                            println!(
                                                "Got error when trying to kill process with PID {}",
                                                one_process.pid
                                            );
                                        };
                                    }
                                }
                                Err(stat_error) => {
                                    println!("Failed to unwrap stat of a process when trying to stop duplicate precesses. The error message was {:?}", stat_error)
                                }
                            },
                            Err(error_msg) => {
                                println!(
                                    "Failed to get a process when trying to stop duplicate precesses. The error message was {:?}",
                                    error_msg
                                );
                            }
                        };
                    }
                }
                Err(me_stat_error) => {
                    println!("Failed to unwrap stat of this process when trying to stop duplicate precesses. The error message was {:?}", me_stat_error)
                }
            }
        }
        Err(me_error) => {
            println!(
                "failed to get the PID of this process due to error {}",
                me_error
            )
        }
    }
}
