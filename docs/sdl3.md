# SDL3 platform

Platform binds SDL3 directly. Window creation calls SDL_SyncWindow before
window_ready. Future window size/fullscreen/state changes must also synchronize
before reporting completion. The current software clear/present path is the R0
shell; it does not implement the shared scene renderer yet.

SDL3 C-layout events translate keyboard, UTF-8 text, floating mouse motion,
wheel, gamepad, focus and quit events into the one core system-event ring.
Text is copied before SDL invalidates its pointer and retains its cursor across
ring backpressure, reserving the final Time slot. Invalid UTF-8 or events over
8,192 bytes are dropped whole and counted. Fractional mouse motion is retained.
Gamepads open by SDL3 instance ID, with fixed load-sized device slots and scoped
hot-unplug cleanup. Physical gamepad validation remains future integration.

Platform event clocks use SDL_GetTicksNS relative to pump creation. Developer
Stopwatch uses the performance counter with a cached frequency. Neither the app
nor game modules read OS/SDL clocks. The rule checker covers SDL3 imports and
aliases as well as SDL2 and raw SDL symbols, including examples.

AudioStream supplies a shared SDL3 device-rate signed 16-bit PCM transport for
the later mixer. It opens a stream at load and uses SDL_PutAudioStreamData;
subsystem references and destruction are independent of window lifetime.
There is no game mixer or sound-event playback in the current shell.

```sh
python3 tools/check_sdl3.py --binary "$QA_CANDIDATE" --owner-profile "$QA_PROFILE" --evidence "$QA_EVIDENCE/sdl3"
cargo build --release -p qa-platform --example audio_stream --features allocation-tracking
timeout 300 env SDL_AUDIO_DRIVER=disk SDL_AUDIODRIVER=disk SDL_AUDIO_DISK_OUTPUT_FILE="$QA_EVIDENCE/pcm.raw" taskset -c "$CORE" target/release/examples/audio_stream
```

The display verifier uses owned Xvfb/Openbox, headless sway/pixman and headless
Weston/pixman. Wayland gets an owned temporary runtime and an explicitly chosen
compositor socket, with no inherited DISPLAY, desktop bus or Wayland socket.
The runtime is removed after recorded PIDs stop. X11 still unsets WAYLAND_DISPLAY
and forces its driver. Both current SDL3 and legacy hint names are set, including
the private disk output file. Candidates and owner settings are copied.

Each display runs 60 warm-up and 600 measured shell frames with time, stdin
ConsoleLine and output dispatch. Xvfb and sway also use real compositor keyboard
holds/repeats and configured bind echoes. The helper waits for initial command
execution, and sway admits its virtual keyboard before pressing a key. Headless
Weston lacks the virtual-keyboard protocol; its input proof is real stdin,
not keyboard movement. Screenshots capture the single owned output/window.

The allocation gate covers the instrumented Rust thread, excluding SDL/driver
allocations. The separate PCM probe measures delivery of a generated 400 Hz
signal and captures private disk audio; it does not prove game cues. Map rendering,
movement, menu and combined gameplay acceptance remain at R3.5 and later gates.
Existing proof-feature input-player code remains a THE-893 release blocker;
all normal qualification candidates exclude that feature.

API references: [SDL3 migration](https://wiki.libsdl.org/SDL3/README-migration),
[SDL_SyncWindow](https://wiki.libsdl.org/SDL3/SDL_SyncWindow),
[SDL_OpenAudioDeviceStream](https://wiki.libsdl.org/SDL3/SDL_OpenAudioDeviceStream).
