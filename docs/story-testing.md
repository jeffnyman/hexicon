# Story Testing (a design note)

Proposed, 2026-11-28, and tried on The Tide Mill (below). Nothing here is decided: what follows is recommended, and the questions at the end are the user's.

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
+ Await 'The ghost is on a fuse the music box sets.'
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
- **`Await`** (`Wait` in the first draft, but that is Hexad's action): a step for what happens on its own: it waits a turn at a time until `until` is true, and fails if `within` turns pass first. Fuses, daemons, and characters' agendas are tested this way.
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
2. **`hexicon test`**: the runner, the report, the exit status, snapshots for `after`, and source lines for failures. game-check could then play a game's scenarios as well as its transcripts. Done, 2026-11-28 (below).
3. **Walkthroughs**, and printing them. Done, 2026-11-28 (below).
4. **The order of events**: `storyEvent`, `happened`, and `happenedBefore`. (`Await` came sooner, in the trial.) Done, 2026-11-28 (below).
5. **Into Hexad**, from the playground, with a page in the guide, if the trial says so.

## The trial (2026-11-28)

Step 1, on The Tide Mill, in the playground (`game-tests/tidemill`): `scenarios.hxn`, the parts (about 400 lines), `scenarios.h`, their templates, and `tests.hxn`, the game's scenarios (190 lines), all in debug builds only.

- **What was tried:** `Scenario` with `after`, `holding`, `startAt`, and `given()`; `Step` with `expect`, `mentions`, `avoids`, and `Check`s inside it; `Await`, with `until` and `within`; an ending caught and checked (`scenarioMemory.ending == ftVictory`); and `SCENARIO <name>` and `SCENARIOS`, which report in the transcript and sum up, and put the game back as it was. Each scenario starts from a snapshot of the start, taken before the first prompt, or of the end of its `after`, with the random numbers as they were then.
- **The suite:** twelve scenarios, of 31 steps and checks. Seven are the way through, each starting where the last ends, to the ending, won, with every point. Five test the puzzles' rules alone, from given conditions: the wheel pit without a light, the stones with no grain, the lever with the wheel still, the rusted windlass, and the flour coming three turns after the stones start, and not sooner.
- **What it caught,** with Tidemill changed three ways and changed back, against game-check's transcript of the walkthrough:

  | Change | Scenarios | The transcript |
  |---|---|---|
  | The yard's description reworded | all pass | 30 lines differ |
  | The flour after two turns, not three | "the flour, in time" fails at "z. z": *Two turns on, it has not come yet* | 4 lines differ |
  | The hopper never set full | "filling the hopper" fails at "put sack in hopper"; the three after it are not run | 26 lines differ |

  The last is the case the proposal is for: the game still said the grain went hissing into the hopper, so its words said the puzzle worked while its state said otherwise. The scenario named the step and why it is there; the transcript showed that the text had changed further on.
- **Found in the game:** the debug build warned that Ada's vocabulary had a pronoun among her nouns (`'Ada; old; aunt woman her'`, one semicolon short), now put right; the walkthrough's transcripts are unchanged.
- **Learned:**
  - `Wait` and `WaitFor` are Hexad's actions, so the timed step is `Await` in the trial.
  - Hexad's typography makes `--` a dash and folds spaces together, so a report written into the transcript uses `====` and quoted spaces.
  - A step may be several commands (`'n. n. d'`), checked after the last: what the game said, in a failure, is all of them.
  - The start has to be taken once the game is running, at the first prompt, and kept when restoring it comes back to the code that took it.
- **Not in the trial:** `hexicon test` and its report and exit status, the source line of a failed check, walkthroughs and printing them, and the record of the story's events. A failure's report names the step, its reason, and what failed (`expect`, or a `Check`'s label).

## Step 2: `hexicon test` (2026-11-28)

`hexicon test [-f <manifest>] [--transcript <file>] [<scenario>...]` builds the game for debugging, plays its scenarios, all or those named, and prints a report:

```
passed   freeing the wheel (3 steps)
FAILED   the flour, in time
           at step 2: z. z
           why: Two turns on, it has not come yet.
           failed: expect is not true, at tests.hxn:182
           the game said: Time passes. Time passes. Flour comes pouring from the spout, and the sack beneath it fills and sags.
not run  bringing Ada her flour, since "grinding the flour" did not pass

8 passed, 1 failed, 3 not run
```

- **The exit status** is 0 if every scenario passed, 1 if any failed or was not run, and 2 if the game could not be built, or did not play its scenarios (a game without the scenario runner, which the report says, with how its output ends).
- **How it plays them:** the game is built into a file of its own beside the manifest's image, which is left alone, and played by `hexicon run` with the random numbers and the clock game-check uses, given the arguments `-hexicon-test=<file>` and `-scenario=<name>` for each scenario named. The scenario runner sees them at the first prompt, plays the scenarios, writes a line for each, one for each failure, and a last line to the file, and quits; `hexicon test` reads the file and deletes both. `--transcript` keeps everything the game printed.
- **Where a check is written:** `definitionPlace(obj, prop)`, a function of the VM's own (`vm.h`, `hexicon-vm/010007`), gives the file and line of a method without calling it, from the line records every build keeps. A failed `expect`, `Check`, or `Await` is reported at its line.
- **game-check** plays the scenarios of each game that has them (a source that defines `name: Scenario`) and wants them all to pass: `SAME tidemill/scenarios (12 passed)`.
- **Tried** on The Tide Mill with the flour's timing broken and with the hopper never filling (above): each failure named its step, its reason, and its line, and the scenarios after the hopper were not run. A scenario named that is not there fails, and so does a game with no scenarios, with exit status 2.
- **Still in the playground:** the scenario runner itself, in Tidemill's folder, until step 5 brings it into Hexad, so `hexicon test` works today for a game that has it among its sources.

## Step 3: walkthroughs (2026-11-28)

`Walkthrough 'name' 'purpose' [scenarios]` plays its scenarios in order from the start of the game, each straight on from where the last ended, with no snapshots between them and their given conditions left out: the way a player goes. The Tide Mill's is the seven scenarios of the way through, 21 steps from the lane to the ending.

- **Playing them:** `walkthrough <name>` and `walkthroughs` in the game; `hexicon test --walkthroughs` plays them after the scenarios, and a name given to `hexicon test` may be a walkthrough's. game-check plays a game's walkthroughs with its scenarios (`SAME tidemill/scenarios (13 passed)`).
- **Notes, not failures:** a scenario whose `after` is not the one before it in the walkthrough, or that has given conditions, which the walkthrough leaves out, is noted in the report (`note: freeing the wheel starts after the tools in the cottage, but follows into the cottage here`). A failure says which scenario's step it was: `at step 1 of "the stones with no grain"`.
- **Printing one:** `hexicon test --print <walkthrough>` plays it and, if it passes, writes it to the output as Markdown, with the report on the error output, so that `> walkthrough.md` keeps the walkthrough alone; one that fails prints nothing, with exit status 1. The game's name and the walkthrough's are the title, each scenario a section with its purpose, and each command a line with why it is there; a wait says how many turns it took when it was played:

  ```markdown
  ## Grinding the flour

  With the wheel turning and the hopper full, the lever starts the stones, and the flour comes.

  - **W. N. PULL LEVER**: The lever puts the stones in gear.
  - **WAIT**, 3 times: The flour takes a while to come down the spout.
  - **TAKE SACK**: The flour is for Ada.
  ```

  The reasons are written for the author, so this is a walkthrough for the author and those who test the game; one for players, in their words, with spoilers hidden, is the question below.
- **Found on the way:** what the game said, in a report, ran sentences together where Hexad's paragraph markup separated them (`You see no lever here.If this was`); the markup is now taken out before the entities are.

## Step 4: the order of events (2026-11-28)

While scenarios play, the story so far is kept: each moment, in order, with its turn.

- **What is kept without asking:** the rooms the player travels to (the room, from `Room.notifyDeparture`, which games do not override, where `travelerEntering`, which they do, would be missed), the achievements awarded (the `Achievement`, or the text a score is given with, from `addToScore`), the fuses that fire (the property they call, `&finishGrinding`, from `Fuse.executeEvent`), and the ending (the `FinishType`).
- **What a game marks:** `storyEvent('Ada says where the key is')`, anywhere in its code. It is defined in a build for release too, and does nothing there, nor outside a scenario.
- **Asking:** `happened(x)`, `happenedTimes(x)`, and `happenedBefore(a, b)` (whether `a` first happened before `b` first did), in an `expect`, a `Check`, or an `Await`. `timesHappened` would have been the name, but Hexad's `Scene` has a property of it, and a name is one thing in a program.
- **The story runs from the start of the game:** a scenario after another goes on from that one's story (kept with the snapshot of its end), a walkthrough from its first scenario; a scenario that starts at the start of the game starts a story of its own, and given conditions are not part of any.
- **The Tide Mill's,** five checks more: knocking is when Ada says where the key is, and that comes before the key's point; the way in is the yard before the kitchen; the flour's fuse fires after the stones start, and once; the dark pit is reached by the yard and the stone floor; and at the end, the seven points came in the order of the story, then the ending, each once. Broken two ways, and put back: without its mark, "into the cottage" failed at the knock, though the game's words were the same, and with the fuse firing twice, "grinding the flour" failed at the turn after it.
- **Found on the way:** a report showed Hexad's quotations without their marks (`<q>`); what the game said is now shown with them.

## Questions for the user

- **Is this worth it**, now the trial has run (above)? Recommended: yes, on to step 2, the runner.
- **The names:** `Scenario`, `Step`, `Await` (`Wait` is Hexad's action), `Check`, `Walkthrough`, `after`, `given`, `expect`, `mentions`, `avoids`; `storyEvent`, `happened`, `happenedTimes` (`timesHappened` is a property of Hexad's `Scene`), `happenedBefore`; and the command, `hexicon test`.
- **Hexad's `Test` and its assertion commands:** kept as they are, as the quick tool for replaying commands while writing; or, once scenarios are in, retired, with the assertions folded into scenarios.
- **Test files in the manifest** (`tests = [...]`), or `#ifdef __DEBUG` for now.
- **A printed walkthrough** (made in step 3, above): how much it should look like one written for players, and whether a scenario may say something to players apart from why it is there for the author.
