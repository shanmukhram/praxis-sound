# Phase 3: your real-hardware check

Everything else in Phase 3 passed on this machine (594 Rust tests, 172 app tests, 11 browser
tests, acceptance test 1, the by-hand exit run with 0 xruns on the silent output). This check is
the one thing only you can do: real speakers or headphones, your ears. Nothing plays until you
press Play. About 10 minutes.

## Start the app

In PowerShell, in the project folder:

```powershell
cd "C:\Users\admin\Desktop\praxis sound"; $env:PRAXIS_HOME = "$env:USERPROFILE\Praxis Sound"; $env:PRAXIS_PACKS = "$PWD\packs"; $env:PRAXIS_KEYCHAIN = "off"; .\target\release\praxis-app.exe
```

`PRAXIS_HOME` keeps sounds in `C:\Users\admin\Praxis Sound`. Without it, the current build tries
`Documents`, which Windows' ransomware protection (Controlled Folder Access) blocks, and the app
closes at once. The next build makes that folder the default and never closes for it.

`PRAXIS_KEYCHAIN=off` keeps typed requests on the free offline planner during the check (with it
on, typed requests use the paid Claude assistant). Building by hand never calls Claude either way.

## Build it by hand

1. **Start a blank sound**: A minor, 16 bars, 4/4, a length of 30 seconds (it solves 128 BPM),
   loop.
2. **Add a layer** -> Felt piano. In Rhythm, say bar 1 as `DON din don din TIN din don din` (or
   enter steps); place pitches in Notes.
3. **Add a layer** -> Soft kick, playing bars 9-16.
4. **Make a take**. The take should show 8 of 8 checks passed.

## Listen for

| # | Do | Listen / look for |
| --- | --- | --- |
| 1 | Press Play (Space), let it loop at least twice | The loop seam from bar 16 back to bar 1: no click, gap or fade |
| 2 | Mute the kick, unmute, solo the piano, unsolo, while playing | Each change heard at once (within a beat), no click |
| 3 | While playing, select bars 13-16 of the piano and ask "softer" | The new take swaps in while playing with no click or gap |
| 4 | While playing, make a hand edit and Make a take | No crackle while the new take renders in the background |
| 5 | Click a bar number; set a loop on the loop strip | Playback jumps there / loops those bars cleanly |
| 6 | Look at the playback line under the Sound panel title | It still reads 0 xruns while playing |
| 7 | Export WAV | The file plays the same in another player |

Tell me "sounds right" (closes Phase 3), or what you heard (a click, a crackle, a lag) and at
which step, and I'll turn it into a fix.
