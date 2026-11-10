# clight

A C and C++ IDE for computers that struggle with the big ones. It's written in Rust and Tauri and sits at around 300 MB of RAM when idle.

It's still early, so expect rough edges. If something breaks, please open an issue and tell me what you were doing.

## Download

Get the latest version from [Releases](https://github.com/blackowlzz/clight/releases). There's an AppImage, a `.deb` and an `.rpm` for Linux, and an installer for Windows. After that, clight updates itself.

On Windows the debugger isn't there yet.

## What you need

CMake and a compiler. Everything else is optional and only turns on if it's installed:

- `clangd` for smart completion, go to definition and rename. It also gives the errors while you type. Without it, completion suggests keywords and names from your open files, and errors come from `gcc` or `clang`
- `gdb` for debugging
- `git` for the commit panel
- `clang-format` for formatting

On Windows clight offers to download clangd, clang-format and the rest for you. You can also find it under Help > Download clangd utils.

On machines with little free memory clangd stays off unless you turn it on in Settings.

## What it does

You get the things you'd expect from an IDE: CMake projects with Debug and Release builds, a real debugger with breakpoints and watches, a terminal, Git, and code navigation and refactoring through clangd. Folders without a CMakeLists.txt work too, it just compiles the file you have open.

There's an optional AI chat too. Connect Ollama, LM Studio, llama.cpp or an online service in Settings > AI, turn the chat on, and it opens in a column on the right with Alt+8. Nothing runs and nothing is sent anywhere until you do that.

Themes and colors are all yours to change in Settings, along with fonts, spacing and the small animations.

## Shortcuts

Mostly the same as CLion. Ctrl+Shift+A finds any command if you forget one.

| Keys | |
| --- | --- |
| Ctrl+P | Open a file |
| Ctrl+Shift+F | Search the project |
| Ctrl+B, Alt+F7 | Go to definition, find usages |
| Shift+F6, Alt+Enter | Rename, quick fix |
| Ctrl+Alt+L | Format |
| Ctrl+Shift+Enter | Complete the statement |
| Shift+Enter, Ctrl+Alt+Enter | New line below, above |
| Ctrl+Shift+U | Toggle case |
| Ctrl+Shift+M | Matching bracket |
| Ctrl+Shift+Backspace | Last edit |
| Ctrl+F9, Shift+F10, Shift+F9 | Build, run, debug |
| F7, F8, F9 | Step in, step over, resume |
| Alt+F12 | Terminal |

Gray suggestions finish the line you are typing, and Tab accepts them. They come from code you already wrote and, when the built-in AI is installed, from the AI as well. Both can be turned off in Settings.

Drag a selection to move it, or hold Ctrl to copy it. Ctrl+wheel changes the font size.

From a terminal you can open the current folder with `clight .` or `cl .`. On the AppImage, turn that on from Help > Install clight and cl Commands. On Windows the installer sets them up, so just open a new terminal afterwards. Inside a Visual Studio Developer Prompt `cl` is the MSVC compiler, so use `clight .` there.

## Building it yourself

You need Rust and the WebKitGTK 4.1 development files.

    cd src-tauri
    cargo run --release -- /path/to/project

The fonts are Inter Tight and JetBrains Mono, both under the SIL Open Font License.
