# Batch 1 evidence (sanitized)

Feasibility batch 1: desktop pet, offline install and webcam resource spike. Disposable spike on branch spike/batch1, never merged.

| File | What it is |
|---|---|
| Batch1_Results_Run1-2.md | Runs 1 and 2, 19 Oct, i7 Windows 10 desktop (16 GB). Camera never started; script bugs. |
| Batch1_Results_Run3_Laptop.md | Run 3, 19 Oct, 8 GB Windows 11 laptop. Sampler bug; camera release and denial checks failed or invalid. |
| Batch1_Results_Run4_Laptop.md | Run 4, 20 Oct, same laptop, build 701207e. DirectShow capture diagnosed as the cause of the camera failures. |
| Batch1_Results_Run5_Laptop.md | Run 5, 20 Oct, same laptop, build 6ef6eb3 (Media Foundation restored). 12 of 12 pass on the laptop. |
| Batch1_Feasibility_Summary.md | Summary prepared for the review, 20 Oct. |

- Build commits: 701207e and 6ef6eb3 on branch spike/batch1 (earlier builds 7523f69 and 2c3b002 are named in the run reports).
- Figures are measured on one 8 GB laptop and one 16 GB desktop only. No 4 GB PC was tested; the minimum PC target is now 8 GB.
- Raw logs and zips stay private with the owner: they contain local usernames and process paths. The reports here have been sanitized; no images or raw log contents are included.
- Status: provisionally accepted by Codex on 20 Oct, not independently verified. See docs/STATUS.md and docs/Decisions.md (DEC19).
