# 0151: The game interview before making a game from a prompt

Status: accepted (2026-11-16)

## Context

The game starter (decision 0150) turns a one-line idea into a game. From a bare prompt the brief
comes from a keyword reader: it guesses the genre, the view, the theme and each asset slot, and
an agent was only told it "should improve" the brief. The owner asked that making a game from
a prompt always asks the person questions about the game first, so the game they meant is the
one that gets made.

Options considered:

1. **Only tell agents to ask** (a skill step). Cheap, but nothing stops a caller from skipping
   it, and the CLI on a terminal still asks nothing.
2. **An interview the engine owns** (chosen): a fixed question table in `marionette-starter`,
   served to agents as an operation and asked by the CLI on a terminal, with answers folded
   into the brief; a bare prompt is refused until the person was asked.
3. **A provider call that writes the questions.** Varies run to run and pays twice: the agent
   calling the engine is already a language model and can reword fixed questions itself.

## Decision

- Questions (`marionette_starter::interview`), in order: `genre` (maze | platformer, each with
  its core loop and controls), `view` (2d | 3d, maze only), `setting`, `player`, `pickups`,
  `enemy`, `goal`, `style`, `difficulty`, `length`, `title`, `extra`. Each carries `why` (what
  it changes), choices or examples, and the guess used when it is not answered (the keyword
  reader's reading of the prompt plus the answers so far).
- Answers are strings by question id. The setting joins the prompt so the keyword reader
  picks its theme (colors, walls, floors, backdrop); the others replace their brief fields.
  Choice answers are checked (`starter.bad_answer`, every problem at once). The seed stays the
  prompt's, so answering does not move the level.
- Operation `game_interview {prompt, answers?}` returns the open questions, the count
  answered, notes for ideas the templates cannot make (racing, shooter, ...), and the next step.
  `game_brief` and `game_make` take the same `answers`.
- `game_make` from a prompt with no `answers` is refused (`starter.interview_needed`) unless
  `skip_interview` says the person declined the questions. A brief is accepted as is: writing
  one is the design step.
- The answers are written to the project's `interview.json`; `extra` (what the templates do not
  build) goes into the handoff notes as a TODO and comes back as `wishes`.
- CLI: `marionette make "<idea>"` asks the open questions on a terminal (Enter keeps the guess,
  a choice by number or name); `--answer id=value` answers up front; `--yes` keeps the guesses.
  Off a terminal with neither, it is refused with the questions listed.

## Consequences

- Focus 1 (modularity): the questions are data in the starter crate; a new template adds its
  choice and slot questions there, and both the CLI and the MCP server follow.
- Focus 2 (tokens): one call returns only the open questions (about 2 KB); answers are short
  strings, not a rewritten brief.
- Focus 3 (performance) and 4 (graphics): unchanged; the interview runs before anything is built.
- Callers that made games from a bare prompt must now pass `answers` or `skip_interview`.
- The questions are fixed in English; an agent rewords them for the person.
