# Minecraft Explorer 1.0.2

Download the single `MinecraftExplorer-1.0-win-x64.exe` from the [Fimel project page](https://studiofimel.com/#/works/minecraft-explorer). [Source code and issues](https://github.com/Ylong4004/Minecraft-Explorer). The app embeds its icons, project MIT license and third-party license texts. Settings → About → Export third-party licenses saves them to the app data directory for offline reading.

Language and theme dropdowns now apply immediately, save automatically, and preserve the latest selection during rapid changes. An original grass cube and magnifier icon is embedded in the EXE and window. Source assets and the optional Pillow generator are in assets/ and scripts/generate_icon.py; running the app needs no external image files or Python.

A native Windows Minecraft Java content manager built with Rust, Slint and SQLite. It indexes worlds, versions, game directories, resource packs, mods, shader packs and valid data packs across multiple sources.

Run `MinecraftExplorer.exe` from any accessible folder. Windows 10/11 x64 and the Microsoft Visual C++ x64 runtime are required; this app does not require Rust or Java. HMCL JAR installations need their own compatible Java runtime. Clean Windows virtual machines have not been tested.

In Settings, add a Minecraft root containing `versions`, or a custom game directory. For the provided installation, choose `D:\PCL2\MinecraftPcl`. Set Language to English and Theme to Dark or Light; both preferences persist. Names, paths and third-party declarations retain their original text.

Search and filter content, view icons and details, add favorites/tags, and use Windows Explorer to locate files. Storage analysis measures real logical bytes, deduplicates overlapping sources, and provides a treemap with category charts. Double-click folders to drill down, use Up to return, and select indexed content to open its list. Small tiles are grouped; skipped paths and analysis limits produce an explicit lower bound. Reanalyze to update the snapshot.

File operations include rename, verified copy/move, directory backup/restore and Windows recycling. Every write requires a preview and explicit confirmation. No overwrite or permanent-delete fallback is supported. Running worlds, protected containers and links are refused. Moves verify the destination before recycling the original. Backups include a checksum manifest and must be outside game directories. Completed batch items remain after cancellation.

The context menu offers file actions and launcher access. F2 opens Rename, Delete opens Recycle, Enter reveals the selection, arrow keys select rows, Ctrl+A selects the list, Ctrl+Shift+A clears it, Ctrl+C copies the path, Ctrl+B opens Backup, Ctrl+F/K focuses Search, F5 refreshes and Esc closes dialogs. Text fields keep their editing shortcuts.

PCL2, PCL CE and HMCL can be opened from the app. To launch a specific version directly, link its BAT / CMD script. Opening a launcher or successfully running a script does not confirm a game launch. Windows UAC handles installations configured to run as administrator. The app does not read login tokens or change launcher settings.

## First use

1. Download `MinecraftExplorer-1.0-win-x64.exe` and run it directly. No extraction or installer is needed. Close an older running app before opening the new version. It can run from any readable folder; app data goes to `%LOCALAPPDATA%\MinecraftExplorer`, outside game folders. Optional documentation ZIPs still contain `MinecraftExplorer.exe`.
2. Open Settings and add the root containing `versions`, such as `D:\PCL2\MinecraftPcl`. Add custom game directories separately and select their associated root.
3. Adding a directory selects it in Space analysis, refreshes the index, and then automatically analyzes that directory. No manual Reanalyze click is required. Cancel keeps the directory and completed results and skips the remaining automatic analysis. Entries appear before all metadata and sizes are ready. “Not measured” means an unfinished measurement, and ≥ means a lower bound. F5 refreshes the index; Reanalyze updates a space snapshot.
4. Open All worlds, search by name or path, and filter by game directory. Click a row for details; Enter reveals it in Explorer. Favorites and comma-separated tags are saved in this app's database.
5. Add a PCL2, PCL CE or HMCL installation in Settings with its executable, working directory and Minecraft root. HMCL JAR also needs a Java executable.
6. Select an item and choose Set mapping. Confirm the version and actual game directory. Shared worlds need a manually selected target. Open selected launcher opens the original launcher for manual selection.
7. To launch from an item, export a BAT / CMD for this exact version, then select it and its working directory in Set mapping. Save and click Run linked launch script. If using PCL's LatestLaunch.bat, verify that the last launch was the intended version and save a dedicated copy; do not bind the overwritten last-launch file. Preserve the original working directory when relative paths require it.

Scripts may contain login credentials; keep them private. The app neither reads nor refreshes script login information. Regenerate a script in the launcher after login or path changes. Clearing the script field and saving removes the binding; choosing another version clears the field. Automated tests run harmless generated marker scripts, and do not verify actual Minecraft launch with a user's exported script.

## Analysis and size updates

Choose a root or game directory in Space analysis and click Reanalyze after the current task finishes. Up to four workers enumerate disjoint subtrees; file count and elapsed seconds appear above the diagram. Double-click directories to drill down, go Up to return, and use linked content navigation. Totals use logical bytes and count overlapping sources once. At most 100,000 drawing nodes are retained; omitted drawing nodes remain included in byte totals. The walker streams entries and refuses reparse points, with a directory-depth limit of 1,024. Unreadable files and skipped links produce lower bounds.

Size scanning reuses completed directory subtree totals across instances, worlds and unpacked packs within one task. Monitoring automatically recalculates affected instance totals. This is ordinary-permission directory enumeration, not direct NTFS MFT access, so speed is not guaranteed to match elevated WinDirStat scans.

App settings and indexes are stored in `%LOCALAPPDATA%\MinecraftExplorer\index.db`, or the directory specified by `MINECRAFT_EXPLORER_DATA`. Operation history is stored beside it under `operations`. Scanning, previews, analysis and duplicate detection do not write game files.

When scanning or monitoring encounters a problem, **View issues** appears in the status bar. Open it to see the affected paths and causes, then fix the problem and refresh. Duplicate messages are combined and only the latest 100 issues from this session are retained. Clear removes the messages without changing files. File-operation errors remain in the operation dialog, and recovery records remain in Cleanup & backup.

See [testing](TESTING.md), [architecture](ARCHITECTURE.md), [launcher research](LAUNCHER_RESEARCH.md) and [licenses](THIRD_PARTY.md). The app's original source is [MIT licensed](../LICENSE); distribution includes Slint attribution and upstream notices.

## Automatic PCL script detection (1.0.1)

Launch the intended version once in PCL2 or PCL CE. Set mapping automatically checks the selected installation’s PCL/LatestLaunch.bat and requires an exact version and game-directory match. A separate copy is saved under the app data launch-scripts directory, keeping a later game launch from replacing the linked script. Review and save the mapping; Auto-detect retries or refreshes a copy after login expires. Manual entries are preserved. Detection handles the verified UTF-8 direct Java command format, refuses ambiguous or variable-based target arguments, and does not parse or refresh login parameters. HMCL and other formats continue to use manual exported scripts. Copies may include the original login parameters; keep them private.
