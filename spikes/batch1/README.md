# HealthierYou feasibility batch 1 (disposable)

Throwaway test app, not product code. It answers three questions on the slowest 4 GB office PC: does a desktop pet stay within the memory, CPU and focus limits; does the app install and run with no internet; and what does a webcam check-in cost in CPU and memory. It does not test posture accuracy, LAN sync or any product feature. This branch is never merged.

What it contains: a 128 px pet animated at 12 frames a second in its own small always-on-top window, a control window with "Start camera test" (MediaPipe pose lite on the CPU at 3 frames a second), and a logger that writes every 5 seconds to `%LOCALAPPDATA%\HY-Spike\logs`. The installer is per-user and bundles the official fixed-version WebView2 runtime, so nothing is downloaded at install or run time. No data leaves the PC.

## Pass and fail criteria

| Test | Pass |
| --- | --- |
| P1 Idle memory | Summed memory of the app and its WebView2 processes, minutes 10 to 30, with Excel and a browser open: at most 150 MB average |
| P2 Idle CPU | At most 2% of the machine over 30 quiet minutes, no one-minute average above 5% |
| P3 No focus stealing | Typing cursor stays in Notepad 10 of 10 pet clicks |
| P4 Response | Click to pet reaction at most 500 ms, 10 of 10 |
| P5 Launch | Double-click to pet visible within 10 s |
| P6 Scaling | Pet sharp, right size and clickable at 100%, 150% and 200% |
| I1 Offline install | Network disconnected, standard user: no administrator prompt, done within 2 minutes |
| I2 Offline run | App opens with the network still disconnected; log shows the bundled runtime |
| I3 Uninstall | Program folder removed by Windows Settings uninstall (the log folder is kept on purpose so you can send it) |
| I4 Warnings | Warning screens recorded for a GitHub download and a USB or LAN copy (no threshold) |
| C1 Webcam CPU | At most 10% average over 10 minutes |
| C2 Webcam memory | Peak summed memory at most 350 MB |
| C3 Camera release | Light off within 2 s of Stop, 5 of 5 |
| C4 Camera unavailable | With camera access off, the app says "Camera unavailable" and the pet keeps working |

A miss is reported with its numbers. Targets change only with the owner's approval.

## Testing checklist (about 2 hours)

Use the slowest 4 GB PC. If it has no webcam, do steps 9 and 10 on the PC whose webcam would be used and note which PC.

1. Write down the PC's processor and Windows edition (Settings > System > About).
2. Download the zip from the GitHub run page, unzip it, and copy the setup file to the desktop. Turn Wi-Fi off and unplug the network cable.
3. Double-click the setup file. Note any blue administrator prompt, any warning screen, and how long installing takes (I1, I4).
4. Open Excel with a normal workbook and a browser with two tabs. Start "HY Spike Batch1" from the Start menu and time how long until the pet appears (P5).
5. Leave everything idle for 30 minutes.
6. At minute 10, open Task Manager > Details, sort by Name, and screenshot the "hy-spike-batch1" and "msedgewebview2" rows (P1 cross-check).
7. After 30 minutes, open Notepad and type. While typing, click the pet. Do this 10 times and count how often the cursor stayed in Notepad (P3).
8. Set display scaling to 150%, then 200% (Settings > Display), check the pet each time, then set it back (P6).
9. In the control window press "Start camera test" and allow the camera if asked. Sit normally for 10 minutes (C1, C2). Press Stop and count the seconds until the camera light goes off. Do Start and Stop 5 times (C3).
10. Turn off camera access (Settings > Privacy > Camera), press "Start camera test", note what appears, then turn access back on (C4).
11. Reconnect the network. Copy the setup file to a USB stick or shared folder, run it from there and note any warning (I4).
12. Uninstall from Settings > Apps (I3).
13. Zip the folder `%LOCALAPPDATA%\HY-Spike\logs` and attach it in the project thread with the screenshot and your notes. The logs hold only timings, memory and CPU figures, process paths and the PC's processor count and RAM; no images, names or health data.
