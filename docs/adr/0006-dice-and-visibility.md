# 0006: Dice, chat and hidden information

Status: Accepted (2026-11-12)

## Decision

### Dice
- The server rolls every die. A client can ask for a roll; it can never supply a result.
- Full dice notation is supported (`2d20kh1+5`, `8d6`, exploding and reroll rules), with one-click advantage and disadvantage on sheet rolls.
- The client plays a 3D dice animation that lands on the server's result.
- The animation uses the dice-box library family (`@3d-dice/dice-box`, MIT), already used on the portfolio site (decided 2026-11-12). It lives in `web/` behind a small interface, the same way `web/media` wraps its video library, and is Milestone 4 work.
- Open point, to settle before Milestone 4: the animation must show the server's numbers, never its own. `@3d-dice/dice-box` decides results with its physics; its sibling `@3d-dice/dice-box-threejs` documents notation that pins each die's face (`2d6@4,2`). Check whether the main package can do the same; if not, use the sibling. If neither lands reliably, the roll card's numbers are authoritative and the animation is decoration.
- Each roll is a roll card in chat showing every die, every modifier and its source, with GM buttons to re-roll, halve or undo.

### Roll visibility
- **Player rolls are always public.** Everyone at the table sees them, so the table polices itself.
- **Only the GM can hide a roll.** The roll-visibility field on a roll request is honoured only when the sender has the GM role; the server forces every other roll to public.
- Private text whispers are not in scope.

### Hidden information
- The GM configures what players see about creatures they do not control. The default is health states only ("Healthy", "Bloodied", "Near death"), never numbers and never the stat block.
- The GM can reveal exact HP, AC or the whole sheet per creature.
- Filtering happens on the server. Hidden data is never sent to a browser that may not see it.

## Why

- Server rolls and server filtering are the only defences that survive someone opening the browser's developer tools.
- Public player rolls were chosen over private rolls deliberately, to discourage cheating.

## Rejected

- **Player private rolls and whispers:** against the communal-policing goal.
- **Client-side hiding:** trivially bypassed.
- **A roll-audit screen:** not wanted. Rolls are still in the event log because undo needs them.
