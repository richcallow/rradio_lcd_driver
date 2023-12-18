#!/usr/bin/env bash

#cd ../rradio
#./run_internet_radio_debug.bash

ssh -t 192.168.0.11 "cd remote_rradio; sudo ./rradio"