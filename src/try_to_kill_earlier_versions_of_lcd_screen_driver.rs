use psutil::process::Process;
pub fn try_to_kill_earlier_versions_of_lcd_screen_driver() -> () {
    let me = procfs::process::Process::myself().unwrap();
    //println!("my pid is {}", me.pid);

    for proc in procfs::process::all_processes().unwrap()
    // only fails if there is no "/proc" folder
    {
        let proc = proc.unwrap();
        if proc.stat().unwrap().comm == me.stat().unwrap().comm && proc.pid != me.pid {
            //.comm should be "rradio"
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
