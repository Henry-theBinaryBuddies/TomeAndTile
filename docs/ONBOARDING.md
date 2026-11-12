# Onboarding: what is a virtual tabletop?

A virtual tabletop (VTT) is a website where a group plays a tabletop roleplaying game together online. Everyone sees the same map, moves their own piece on it, rolls dice everyone can see, and usually talks over video. One person, the game master (GM), runs the game and sees more than the players do.

Tome & Tile is a VTT. The two best-known ones are **Roll20** and **Foundry VTT**. Watching them in use is the fastest way to understand what we are building.

## Words you will hear

| Word | Meaning |
|---|---|
| GM / DM | Game master. Runs the game, controls the monsters, sees hidden information. |
| Scene | One map with everything placed on it. |
| Token | A piece on the map standing for a character or monster. |
| Character sheet | A character's numbers and abilities. In a VTT you click things on it to roll. |
| Initiative | The turn order in a fight. |
| Attack roll | A d20 plus a bonus, compared with the target's Armor Class (AC) to see if you hit. |
| Saving throw | A d20 the *target* rolls to resist an effect, compared with a Difficulty Class (DC). |
| Fog of war | Parts of the map players have not seen yet are hidden. |
| Dynamic lighting | The map shows each player only what their character could see, given walls and light. |
| Automation | The VTT does the bookkeeping: rolls the save, works out the damage, lowers the hit points. |

## Watch list

These were found by web search on 2026-11-12. Claude has not watched them; the titles are as listed on YouTube. Some are a few years old, so screens may look different from the current versions. To make this a real YouTube playlist, open each one and use **Save** to add it to a playlist on your account.

### 1. What playing in a VTT looks like (start here, about 25 minutes)

| Video | Why watch it |
|---|---|
| [Player quick guide in 3 minutes for Foundry VTT](https://www.youtube.com/watch?v=3NoK9wXWzh0) | The shortest possible look at the player's screen. |
| [10 MINUTE Foundry PLAYER tutorial](https://www.youtube.com/watch?v=24mciO_P8FI) | Moving a token, opening a sheet, rolling. This is our core loop. |
| [Foundry VTT Player Basics 2024](https://www.youtube.com/watch?v=ibMQVYeCy_Q) | A second take on the same screen, with the interface explained. |

### 2. What the GM sees

| Video | Why watch it |
|---|---|
| [Foundry VTT DM Beginner Crash Course Tutorial](https://www.youtube.com/watch?v=jLnRazPL-J0) | Setting up a map, placing tokens, running a fight. |
| [Roll20 DM Beginner Crash Course](https://www.youtube.com/watch?v=L-1mpJDbKH8) | The same job in the other big VTT. Notice what differs. |
| [FULL WALKTHROUGH TUTORIAL of Foundry (The Rules Lawyer)](https://www.youtube.com/watch?v=LWLrk5cinQU) | Long and thorough. Skim it; it uses Pathfinder, not D&D, but the tabletop is the same. |

### 3. Character sheets you click to roll

| Video | Why watch it |
|---|---|
| [Roll20 Tutorial: Using the D&D Character Sheet](https://www.youtube.com/watch?v=G8u9SaW-1Qs) | Clicking an attack on a sheet and seeing the roll in chat. Most relevant to `web/`. |

### 4. Combat automation (the part that makes Tome & Tile different)

| Video | Why watch it |
|---|---|
| [Foundry VTT - Tips: Automated Combat In Action](https://www.youtube.com/watch?v=uaXXSPZxOsY) | Click an ability, click a target, and the VTT does the rest. This is our goal. |
| [Train Fight - D&D 5e - Foundry VTT - Combat Automation Example](https://www.youtube.com/watch?v=yTjnZfP8h0k) | A whole automated fight, start to finish. |
| [Module Tutorial: MidiQOL Part 2](https://www.youtube.com/watch?v=KO8j2JUelu4) | How Foundry's automation add-on handles damage, saves and immunities. |
| [Foundry VTT: Speeding Up DnD Combat: Automating a World From Scratch](https://www.youtube.com/watch?v=WKH2nPSestk) | How much setup automation needs in Foundry. Ours should need none. |

### 5. Walls, lighting and fog (built after the class; watch for context)

| Video | Why watch it |
|---|---|
| [How to upload maps, add walls, lighting and ambient sounds in Foundry VTT](https://www.youtube.com/watch?v=dN7RwF9LQGU) | Uploading a map and drawing walls. Relevant to the map importers. |

## What to watch for your area

| Person | Watch first | Then |
|---|---|---|
| @aalemany00 (importers, assets, video client) | Section 1, then the map upload video in section 5 | Section 2 |
| @Danny-Defeato (UI shell, sheets, panels) | Sections 1 and 3 | Section 4 |

## Try one yourself

Roll20 has a free tier and runs in the browser. Twenty minutes creating a game, dropping a token on a map and rolling `/roll 1d20+5` in chat will teach more than another video.

## Then read

1. [ARCHITECTURE.md](../ARCHITECTURE.md): how Tome & Tile is put together.
2. [SETUP-WINDOWS.md](SETUP-WINDOWS.md): getting your machine ready.
3. The root [CLAUDE.md](../CLAUDE.md) and the one in your directory: what your Claude may and may not do.
4. [ADR 0013](adr/0013-scope-and-schedule.md): the schedule and who does what.
