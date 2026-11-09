# Roadmap

Keitora is in early development, and nothing has been released yet. This page shows where the project is going and how far along it is. If you want to contribute, the detailed task list is in [docs/roadmap.md](docs/roadmap.md).

✅ Done · 🚧 In progress · ⏳ Planned

## Version 1: Windows

The first version runs on Windows 10 and 11. It's built in seven stages, in this order.

| | Stage | What you get |
|---|---|---|
| ✅ | **Foundations** | Project setup, automated checks on every change, and the design of every screen. |
| 🚧 | **Core rules** | How the timer works, and how Keitora decides which sites and apps to block. |
| ⏳ | **Background service** | Blocking that keeps working when you close the app, and a session that continues after a restart. |
| ⏳ | **Desktop app** | The timer, your blocklists, settings and history, a tray icon and notifications, in English and Spanish. |
| ⏳ | **Website blocking** | An extension for Chrome and Edge that shows a block page instead of distracting sites. |
| ⏳ | **Strict mode, hardened** | A strict session that holds up against closing the app, ending it from Task Manager, changing the clock or uninstalling. |
| ⏳ | **Installer and first release** | A signed installer, the extension in the Chrome and Edge stores, and a user guide. |

The first releases will be betas. A stable release only comes after an independent, human security review.

## After version 1

- **Sync**, opt-in and self-hostable: your blocklists and sessions on all your devices. Nothing leaves your computer unless you turn it on.
- **Android**
- **iOS**

## Not planned for version 1

- Firefox
- Blocking at the network level, for every program at once
- Recurring schedules
- Detailed statistics

## Good to know

Strict mode is meant to stop you from giving in to a distraction, not to stop a determined administrator: someone with administrator rights on the computer can always get around it. The user guide will explain exactly what it does and doesn't cover.

## Follow along

Each stage is a [milestone on GitHub](https://github.com/CornelioDev/keitora/milestones), F0 to F6 in the order above. To help, start with [CONTRIBUTING.md](CONTRIBUTING.md).
