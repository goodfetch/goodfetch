# Kennel spike (throwaway)

Throwaway code for [goodfetch#1915](https://github.com/goodfetch/goodfetch/issues/1915). It is not the Kennel. It answers, on the owner's Mac, what the Linux server cannot, and is then deleted. It already type-checks for `aarch64-apple-darwin`; it has never run.

Needs Rust and the Xcode command line tools. Each run prints timestamped lines: keep the terminal output.

## Steps (about 15 minutes)

1. **Bare binary.** Run `./run.sh terminal`.
   - Does the disc icon appear in the menu bar?
   - Does a Dock icon flash or stay?
   - Click the icon. Does the first line read `Refreshed on click: tick N`, with N matching the log's `click down` line?
   - Keep the menu open for 10 seconds. Does it stay open? Does `Live: tick N` count up while it is open, or only change after you reopen it?
   - Right-click. Does it open the same menu?
   - Choose **Send a test desktop notice**. The log should say notices are off outside a `.app`.
   - Quit from the menu.
2. **App bundle.** Run `./run.sh app`.
   - Repeat the icon and Dock checks.
   - Choose **Send a test desktop notice**. Does macOS ask for permission? Allow it. Does a banner appear?
3. **Rebuild.** Change any character in a comment in `src/main.rs`, then run `./run.sh app` again.
   - Read the line `notice authorization at launch: …`. Does it say `authorized`?
   - Send another notice. Does a banner appear without a new prompt?
4. **Window.** Choose **Open the details window**.
   - Does a window show and redraw about once a second?
   - Close it. Does the log say the window process exited?
5. **Measurements.** In a second terminal, while the spike runs with the menu closed and nothing touched, run `./measure.sh 60`. Then open the window and run `./measure.sh 60` again.
6. **Notch.** Only on a Mac with a notch.
   - Choose **Check whether the icon is visible** while the icon shows.
   - Fill the menu bar until macOS hides the disc. Wider apps' menus help, or many menu-bar icons.
   - Restart `./run.sh app`; the spike checks 3 s after launch.
   - Copy both `icon check` lines.

## Second pass

The first pass ([results](https://github.com/goodfetch/goodfetch/issues/1915#issuecomment-6578895595)) left three questions open.

1. **Notices through LaunchServices.** Run `./run.sh app-open`, which launches the `.app` with `open` as Finder would, and send a test desktop notice. Is a permission prompt shown, a banner shown after Allow, and does the grant survive a rebuild (step 3, with `app-open`)?
2. **A human click.** A person clicks the icon once, quickly. Does the menu open and stay open?
3. **Window memory, glow against wgpu.** With `./run.sh app`, open the window and run `./measure.sh 60`. Quit, then run `SPIKE_RENDERER=wgpu ./run.sh app`, open the window, and run `./measure.sh 60` again. Keep both runs' `target/footprint-<pid>.txt` for the window's pid.

The `rust-toolchain.toml` pins 1.99.0, which eframe 0.37 needs.

## What to send back

The whole terminal output of steps 1 to 6, both `measure.sh` outputs, and a yes or no for each question above. Note the macOS version (`sw_vers`) and the Mac model.
