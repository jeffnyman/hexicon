# Testing a Story

A game under construction breaks in ways that are hard to see. A puzzle stops being solvable because a lock changed; a timed event comes a turn early; a clue moves and a scene loses it. Playing the whole game again after every change finds some of this, slowly. Hexad's scenarios find it every time: the key points of each way through the game, played, with what must be true at each, and why each command is there.

This page uses *The Well House*, a game small enough to read whole: the well house is on fire, its door is locked, the key is in a bird's nest, and a bell rope down in the dark cellar brings the farmer at a run.

## Scenarios

The scenarios are an extension of Hexad's, which a game names in its manifest:

```toml
libraries = ["system", { name = "hexad", include = ["extensions/scenarios"] }]
sources = ["start"]
tests = ["tests"]
```

A game's scenarios go in a source file of their own, which its manifest names among its `tests`: a build for debugging has its tests, and `hexicon test` builds one, and a build for release, which players get, leaves them out. A scenario says how to win, and why.

A scenario is a name, what it is for, and its steps, the objects inside it, played in order. A step is a command, as a player would type it, and why it is there:

```hexicon
getIn: Scenario 'getting in'
    'The well house is locked, and the key is in the apple tree.'
;
+ Step 'look in nest' 'The nest is where the key is.'
    mentions = 'iron key'
;
+ Step 'take key' 'Seen, it can be taken.'
    expect = (ironKey.isIn(me))
;
+ Step 'unlock door with key' 'Getting in is the first point.'
    expect = (!wellDoor.isLocked && gotIn.achieved)
;
+ Step 'n' 'The door opens on the way in.'
    expect = (me.isIn(wellHouse))
;
```

A command can be several, as a player types them (`'take lamp. light lamp'`); what the step checks is checked after the last.

## What a step checks

- **`expect`**: an expression that must be true after the command. It is code, checked by the compiler, so a misspelled or renamed object is found when the game is built, not when the scenario is played.
- **`mentions`** and **`avoids`**: text the command's output must and must not contain, or lists of texts. They ignore case, markup, and line breaks, so restyling or rewording around them does not fail them. Most checks should be on the game's state, with `expect`: what the game says changes as it is written, and what it does should not.
- **A `Check`**, inside a step: an expression with a name of its own, for a step that checks more than one thing and should say which failed.

## Where a scenario starts

A scenario starts at the start of the game, or, with `after`, where another one ends. *The Well House*'s second scenario goes on from where the first ends, inside the well house with the key:

```hexicon
ringBell: Scenario 'ringing the bell'
    'The bell is down in the dark cellar, and it brings the farmer.'
    after = getIn
;
```

To test one thing without playing up to it, a scenario sets its starting conditions instead: `startAt`, where the player is put, `holding`, a list of what the player is given, and `given()`, anything else, set before its first step:

```hexicon
darkCellar: Scenario 'the cellar without the lamp'
    'Without a light the cellar is too dark to find the rope.'
    startAt = wellHouse
;
+ Step 'd' 'The trapdoor opens on the way down.'
    expect = (me.isIn(cellar) && !cellar.isIlluminated)
;
```

Each scenario is played from its start again, however the one before it went, and the random numbers are the same each time.

## Waiting for what happens

What happens on its own, a fuse, a daemon, a character arriving, is waited for with an `Await`, a turn at a time, until its `until` is true; it fails if `within` turns pass first. An ending is caught, not shown, and `scenarioMemory.ending` is how the game ended:

```hexicon
+ Await 'The farmer comes running.'
    until = (scenarioMemory.ending == ftVictory)
    within = 2
;
```

## The story so far

While scenarios are played, Hexad keeps the story so far: the rooms the player travels to, the achievements awarded, the fuses that fire (as the property they call), and the ending. A game marks moments of its own with `storyEvent`:

```hexicon
            // A moment the scenarios can ask about.
            storyEvent('the bell rings');
```

It does nothing outside a scenario, nor in a build for release. A check can then ask `happened(x)`, `happenedTimes(x)`, or `happenedBefore(a, b)`, whether `a` first happened before `b` first did:

```hexicon
++ Check 'the bell, then the end'
    test = (happenedBefore('the bell rings', ftVictory))
;
```

The story runs from the start of the game: a scenario after another goes on from that one's story, and given conditions are not part of it.

## Playing them

`hexicon test`, in the game's folder, builds the game for debugging, plays every scenario, and says how they went:

```report
passed   getting in (4 steps)
passed   ringing the bell (4 steps)
passed   the cellar without the lamp (1 step)
passed   walkthrough: the way through (8 steps)

4 passed, 0 failed, 0 not run
```

A failure says which step failed, why the step is there, what failed and on which line it is written, and what the game said. This guide's first version of *The Well House*'s first scenario expected LOOK IN NEST to take the key, and was told otherwise:

```
FAILED   getting in
           at step 1: look in nest
           why: The nest is where the key is.
           failed: expect is not true, at tests.hxn:12
           the game said: In the nest you see an iron key.
not run  ringing the bell, since "getting in" did not pass
```

A scenario that starts after one that failed is not run. The exit status is 0 if everything passed, 1 if not, and 2 if the game could not be built or did not play its scenarios, so a build, or a check before every change is kept, can stop on a failure. Named scenarios play alone (`hexicon test "ringing the bell"`, with those it starts after), and `--transcript <file>` keeps everything the game said.

In the game itself, built for debugging, `scenarios` plays them all and `scenario <name>` one, reporting as they go; the game is put back as it was afterwards.

## Walkthroughs

A walkthrough is scenarios in order, played from the start of the game, each straight on from the last, without their given conditions: the way a player goes.

```hexicon
wayThrough: Walkthrough 'the way through'
    'From the garden to the farmer, at a run.'
    [getIn, ringBell]
;
```

`hexicon test --walkthroughs` plays the walkthroughs after the scenarios. A scenario in a walkthrough that does not follow on from the one before it, or that has given conditions, is noted. `hexicon test --print <walkthrough>` plays one and, if it passes, writes it in Markdown, each scenario a section and each command with why it is there:

```walkthrough
## Ringing the bell

The bell is down in the dark cellar, and it brings the farmer.

- **TAKE LAMP. LIGHT LAMP**: The cellar is dark.
- **D**: The cellar is under the trapdoor.
- **PULL ROPE**: The rope rings the bell.
- **WAIT**, 2 times: The farmer comes running.
```

The reasons are the author's, so this is a walkthrough for the author and those who test the game; since the tests keep it true, it is a good start on one for players.
