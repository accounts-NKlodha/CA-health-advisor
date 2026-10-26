# HealthierYou office test kit

This kit runs the office tests on two Windows PCs, one step at a time. You start it by
double-clicking a file. It tells you, in large text, whenever it needs you. At the end it
makes one zip file on the Desktop for you to upload.

- **PC1** is the office server PC. It runs the server and the 8-hour soaks.
- **PC2** is a staff PC with a webcam. It runs the camera tests.

## 1. Get the kit

1. Open the repository on GitHub, then **Actions**, then the **ci** workflow.
2. Pick the newest run on **main** with a green tick. Scroll down to **Artifacts** and
   click **office-test-kit**. You need to be signed in to GitHub.
3. You get `office-test-kit.zip`. Copy it to both PCs, right-click it, choose
   **Extract All**, and open the `HealthierYou-office-test-kit` folder.

**These are test builds.** CI builds them on every change. They have no code signature,
so Windows may say "Windows protected your PC" or "Unknown publisher". Click **More info**,
then **Run anyway**. Never turn off a Windows protection. If Windows blocks the file with
no way past it (Smart App Control, or an office policy), note what it said. That is a
finding for the report.

The kit installs nothing extra to trust these builds. Their update keys exist only for that
CI run, so a test build can never update a real install, or be updated by one.

## 2. Before you start

- Both PCs are on the office network, and the network is set to **Private** (or is a work
  domain network), not Public.
- **PC1:** you need an administrator password. PC1 also needs a second drive besides C:. If
  it has none, keep a USB stick plugged in for the whole test.
- **PC2:** a webcam, built in or USB. Sign in as the normal office user, not an
  administrator. The kit still asks once for an administrator password for the network
  capture.
- Keep both PCs plugged in. The kit keeps a PC awake while a long test runs.

## 3. Run it

1. On **PC1**, double-click **`Run-PC1-Server.cmd`**. Windows asks once for administrator
   rights: click **Yes**. A second window opens, "HealthierYou test kit - ADMINISTRATOR".
   Leave it open.
2. When PC1 shows **Pair PC2**, go to PC2 and double-click **`Run-PC2-Camera.cmd`**. Then go
   back and forth between the PCs as the screens tell you.
3. Whenever something stops the kit (a restart, sleep, a sign-out, a closed window),
   double-click the same file again. It carries on at the step it had not finished. After
   a sign-out, it starts again by itself when you sign back in.

## 4. What you will be asked to do

| PC | Step | Time | What you do |
| --- | --- | --- | --- |
| PC1 | Check the PC and the kit | 1 min | nothing |
| PC1 | Install the server | 3 min | plug in a USB stick, only if asked |
| PC1 | Admin passphrase | 2 min | type a passphrase (12 characters or more) twice, and write it down |
| PC1 | Install HealthierYou | 2 min | click **Run** if Windows asks |
| PC1 + PC2 | Pairing | 30 to 45 min | sign in to the admin page, add the user **kit-pc2**, make a slip, copy it to PC2, compare the six words and the code, then click **Confirm** |
| PC1 | 8-hour soaks | 8 to 16 h | nothing; leave PC1 on with both windows open |
| PC2 | Check the PC | 1 min | nothing (it finds the Windows version, memory and webcams) |
| PC2 | Guided install | 20 to 30 min | double-click the installer, answer yes/no questions, change the display scale when asked |
| PC2 | Network capture and pairing | 30 to 40 min | in the ADMINISTRATOR window: paste the slip, use the PC name it gives you, read the six words to whoever runs PC1 |
| PC2 | Start at sign-in | 5 min | sign out, sign in, then put the PC to sleep and wake it |
| PC2 | 2-hour camera soak | 2 to 2.5 h | quit HealthierYou from the tray when asked, allow the camera, work at the desk, lock the PC once, and turn the camera off at the end |
| PC2 | USB webcam soak (only if a second webcam is found) | 2 to 2.5 h | the same, with the USB webcam |
| PC2 | Desk sessions | 45 min each, up to 10 | follow the cues on screen. Do as many as you can. Type **l** to stop and carry on another day |
| PC2 | Overnight soak (only on Windows 10 or about 4 GB) | 8 h | leave PC2 on overnight |
| PC2 | Uninstall check | 10 min | uninstall HealthierYou from Settings, leaving "Also delete my data" unticked |

You can type **s** to skip a step, but a skipped step leaves a gap in the results.

## 5. At the end

The kit shows a table of every step, with PASS, FAIL, INCONCLUSIVE or SKIPPED. It then
makes **`HealthierYou-results-PC1-<date>.zip`** (or `-PC2-`) on the Desktop.
**Upload that file in the project thread.**

- A test passes only when its own report has the evidence. INCONCLUSIVE means something was
  missing, and the table says what. Fix it, then type that step's id (for example `P2-05`)
  to run it again.
- The zip holds only text reports. User names, PC names, folder paths, network addresses
  and the pairing label are replaced before the zip is made. A file that still holds one is
  left out, and so are pictures and raw network captures.

## 6. Afterwards

- To remove the test builds: go to Windows Settings > Apps > Installed apps, and uninstall
  **HealthierYou Server** (PC1) and **HealthierYou**.
- The kit keeps its notes in `%LOCALAPPDATA%\HealthierYou-TestKit`. Its administrator steps
  use `C:\ProgramData\HealthierYou-TestKit-Admin`, which only administrators can change. On
  PC2 that folder also holds the raw network capture files, and only administrators can
  open them. Never share those. Delete both folders when the results are uploaded.
