# Story Testing (a design note)

Proposed, 2026-11-28. Nothing here is decided: what follows is recommended, and the questions at the end are the user's.

## The problem

A game under construction breaks in ways unit tests do not see. A puzzle stops being solvable because a door's lock changed. An event fires a turn early, or after the thing it was meant to warn of. A message moves to another object and a scene loses its clue. What an author needs is a regression suite for the story: the key points of each path through the game, played, with the state checked at each point, and the reason each command is there.

The pieces, from the smallest up:

- **Checks.** Conditions on the game's state (the trapdoor is open, the player is in the cellar, the score rose by five) and, where the words matter, on what the game said. A check on state survives rewriting the prose; a check on text is for the few places where a particular thing must be said.
- **Given conditions.** A starting state set up rather than played to: the player in the kitchen, holding the lamp, lit. They make a scenario short, and test one thing.
- **Steps.** A command, why it is there, and the checks that must hold after it.
- **Scenarios.** A starting state, and steps. A scenario can start where another ends: that is how they compose.
- **Walkthroughs.** Scenarios in order, from the start of the game, each starting where the last ended, with no given conditions: proof that the game can be played through as a player would.

## What Hexicon has now

**Hexad's `Test`** (`debug.hxn`, from adv3Lite, in debug builds only). A test is a name and a list of commands, with things to put in the player's hands (`testHolding`) and a place to put the player (`location`) first. `test <name>` plays it; a test's list can name other tests; `testall` plays every one; `list tests` lists them. A test can restart the game first (`restartBeforeTest`) and put back the state it started from after (`restoreStartStateAfterTest`). And there are assertions, written as commands among the others:

- `assert <expression>`, which the game compiles and evaluates as it runs (`Compiler.eval`);
- `assertMsg <text>`, which looks for the text in what the commands before it printed;
- `assertPlayerInRoom`, `assertPlayerHasItem`, `assertPlayerLacksItem`, `assertPlayerRoomHasItem`, `assertPlayerRoomLacksItem`.

A failure is printed, counted, and stops the test, unless `testall nostop`.

That is most of the machinery. What it lacks:

- **No reasons.** A command list says what is typed, never why.
- **Checks are strings.** `assert` compiles its expression when the test runs, so a misspelled or renamed object is found only then, as a failed assertion, not as a compiler error.
- **Checks are commands**, in the same list as the player's, so a test reads as a mixture of play and bookkeeping.
- **No report.** A test prints into the transcript, and a person reads it. There is no summary, nothing a build can act on, and no way to run the tests from outside the game.
- **Composition is by name**, in strings, and a test that builds on another replays it.

**game-check's transcripts** compare everything a script makes the game say with a recording. They catch any change at all, which is their strength and their weakness: reword one description and every transcript through that room fails, and the failure says what changed, not whether it matters. They stay, for Hexicon's own regression testing and for any author who wants that; scenarios are for "what matters still holds".

## Recommended: scenarios, in the game's own language

Scenarios are objects in the game's source, as `Test`s are, in debug builds only. The checks are expressions the compiler checks, so renaming `trapdoor` breaks the scenario at build time.

A sketch, in Hexad's style; the syntax is to be settled by trying it:

```
lampScenario: Scenario 'get the lamp'
    'The lamp is in the shed, and everything after needs light.'
;
+ Step 'n' 'The shed is north of the garden.'
    expect = (me.isIn(shed))
;
+ Step 'take lamp' 'The lamp is the only light in the game.'
    expect = (lamp.isIn(me))
    mentions = 'brass lamp'
;

cellarScenario: Scenario 'open the cellar'
    'The cellar is where the second act starts.'
    after = lampScenario
;
+ Step 'pull ring' 'The trapdoor is under the rug, and the ring opens it.'
    expect = (trapdoor.isOpen)
;
+ Step 'down' 'The cellar is dark: this checks that the lamp lights it.'
    expect = (me.isIn(cellar) && cellar.isIlluminated)
    avoids = 'pitch dark'
;

ghostScenario: Scenario 'the ghost appears'
    'The ghost must come once the music box has played, within three turns.'
    given()
    {
        me.moveInto(nursery);
        musicBox.hasPlayed = true;
    }
;
+ Wait 'The ghost is on a fuse the music box sets.'
    until = (ghost.isIn(nursery))
    within = 3
;

Walkthrough 'the whole game'
    [lampScenario, cellarScenario, cryptScenario, endingScenario]
;
```

### The parts

- **`Scenario`**: a name, what it is for, and its steps, which are the objects inside it (`+`), in order.
  - `after`: the scenario this one starts where it ends. Without it, a scenario starts at the start of the game.
  - `given()`: the given conditions, set up after the start state is reached and before the first step. Hexad can offer the common ones as properties too, as `Test` does: `holding = [lamp]`, `location = kitchen`.
- **`Step`**: the command, as a player types it (several, with periods, as the parser takes them), and why.
  - `expect`: an expression that must be true after the command. A step can have several checks with names of their own, objects inside it (`++ Check 'the lamp lights it' test = (cellar.isIlluminated)`), when one is not enough to say what failed.
  - `mentions`: text the command's output must contain; `avoids`: text it must not. Both ignore case, markup, and line breaks, so restyling or rewrapping does not fail them.
- **`Wait`**: a step for what happens on its own: it waits a turn at a time until `until` is true, and fails if `within` turns pass first. Fuses, daemons, and characters' agendas are tested this way.
- **`Walkthrough`**: scenarios in order, played from the start of the game, each where the last ended. Their given conditions are left out, since a walkthrough proves the real path; a scenario whose `after` is not the one before it in the walkthrough is reported.

### The order of events

Steps check things in order, which covers most of "this happens before that". For what happens between steps, or where the order of two things is the point, Hexad keeps a record of the story's events while scenarios run:

- what Hexad knows of itself: the player's moves, score awards, achievements, fuses that fire, the ending;
- and what the game marks: `storyEvent('the bell rings')`, a call that does nothing outside a scenario run.

A check can then ask `happened('the bell rings')`, `happenedBefore('the bell rings', 'the door opens')`, or how many times something happened. This is the part most likely to change once it is tried.

### Running them

- **In the game** (a debug build): `scenario <name>` plays one, `scenarios` plays them all, and `walkthrough <name>` plays a walkthrough, each reporting in the transcript as it goes, as `test` does now.
- **From outside: `hexicon test`**, the main way. It builds the game for debugging, plays each scenario without a display, and prints a report:

  ```
  get the lamp ............ passed (2 steps)
  open the cellar ......... FAILED at step 2: down
      why:      The cellar is dark: this checks that the lamp lights it.
      expected: me.isIn(cellar) && cellar.isIlluminated   (scenarios.hxn:31)
      the game: It is pitch dark. You might be eaten by something.
  the ghost appears ....... passed (waited 2 turns)

  2 passed, 1 failed
  ```

  Its exit status says whether everything passed, so a build, or a GitHub workflow, can stop on a failure. `hexicon test <names>` plays some, and `--walkthroughs` plays the walkthroughs too.
- **The same every time.** Random numbers are fixed for each scenario (`--norand`, as game-check runs), and so is the clock.
- **Each scenario starts clean**: from a restart, or, with `after`, from a snapshot taken where that scenario ended (`snapshot.hxn`, which the retry and reversion extensions already use), so a chain of ten scenarios is not played ten times over. A scenario whose `after` failed is reported as not run, not as failed.
- **Where a check failed** is a line of the game's source: a debug build records which line each part of the program came from, so the report can name the line of the `expect` that was false, without the compiler keeping the expression's text.

### A walkthrough, printed

A walkthrough knows each command and why it is there, so it can print as a commented walkthrough: the scenarios as sections, their purposes as introductions, and each command with its reason. `hexicon test --print <walkthrough>` writes it. A game's walkthrough for players, or the start of its hints, is then something the tests keep true.

### Where scenarios live

In source files of their own, listed in the manifest as tests, which a build for release leaves out and a build for debugging takes in (`tests = ["scenarios.hxn"]`). That is a small change to the manifest and the compiler. Without it, an author would wrap them in `#ifdef __DEBUG`, which works today but is easy to forget.

### What is not proposed

- **Branching transcripts** (Inform's Skein): recordings of every path, compared word for word. game-check's transcripts already are the word-for-word kind; scenarios are the other kind.
- **Generating tests**, or exploring a game automatically for dead ends. Worth having one day, and a different tool.
- **Checks on the rich display's markup.** game-check's `.doc` documents do that.

## How to get there

1. **A trial, in the playground.** The parts above (`Scenario`, `Step`, `expect`, `mentions`, `given`, `after`, the in-game `scenario` command) as a Hexad extension, used on one game there, with a few scenarios written for it, including one that is made to fail. The question it answers: does a game's suite read well and catch what breaks?
2. **`hexicon test`**: the runner, the report, the exit status, snapshots for `after`, and source lines for failures. game-check could then play a game's scenarios as well as its transcripts.
3. **Walkthroughs**, and printing them.
4. **The order of events**: `storyEvent`, `happened`, `happenedBefore`, and `Wait`.
5. **Into Hexad**, from the playground, with a page in the guide, if the trial says so.

## Questions for the user

- **Is this worth it**, after the trial? The trial is small enough to answer that before most of the cost.
- **The names:** `Scenario`, `Step`, `Wait`, `Walkthrough`, `given`, `expect`, `mentions`, `avoids`; and the command, `hexicon test`.
- **Hexad's `Test` and its assertion commands:** kept as they are, as the quick tool for replaying commands while writing; or, once scenarios are in, retired, with the assertions folded into scenarios.
- **Test files in the manifest** (`tests = [...]`), or `#ifdef __DEBUG` for now.
- **A printed walkthrough**: wanted, and how much it should look like one written for players.
- **Which game to try it on.** A playground game with a few paths and a timed event would show the most: Tidemill, or The Golden Skull.
