# The prototype's check, inside the secure wadspace (an autostart hook:
# labwc runs it with /bin/sh as abc; prototype.sh up --check mounts it, a
# real wadspace never has it). It waits for the host to drop `play` into the
# project folder (a host folder over virtio-fs), then plays a tone through
# the VM's PulseAudio (what Selkies streams), records the null sink's monitor
# back, and writes what it saw next to it, for the host to read.
(
  P=/config/Desktop/Project
  for _ in $(seq 600); do [ -f "${P}/play" ] && break; sleep 0.5; done
  [ -f "${P}/play" ] || exit 0
  {
    echo "kernel: $(uname -r)"
    echo "vcpus: $(nproc)"
    echo "memory: $(awk '/MemTotal/ {print $2 " kB"}' /proc/meminfo)"
    echo "pulse: ${PULSE_SERVER}"
    echo "saw play after host wrote it: yes"
  } > "${P}/guest.txt"
  gst-launch-1.0 -q audiotestsrc freq=440 volume=0.6 num-buffers=650 ! audioconvert ! pulsesink > "${P}/tone.log" 2>&1 &
  sleep 1
  gst-launch-1.0 -q pulsesrc device=output.monitor num-buffers=60 ! audioconvert ! wavenc ! filesink location="${P}/monitor.wav" >> "${P}/tone.log" 2>&1
  echo "recorded: $(stat -c %s "${P}/monitor.wav" 2> /dev/null) bytes" >> "${P}/guest.txt"
  foot --title krun-check sh -c "cat ${P}/guest.txt; echo; echo playing a tone through the VM's PulseAudio; sleep 60" &
) > /dev/null 2>&1 &
