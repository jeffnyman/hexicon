# Actions and Their Handlers

Every command the player gives is an **action**, done to the things it names. This page is about how a thing decides what an action does to it, and how a game adds actions of its own. Its example, *The Belfry*, is in [examples/belfry](examples/belfry); each piece of code here is part of it.

## How a command is carried out

TAKE LAMP is the action Take, with the lamp as its **direct object**. PUT LAMP IN BOX is PutIn, with the lamp as its direct object and the box as its **indirect object**. JUMP has no objects at all.

For each object, the action asks the object's **handler** for it, in stages:

| Stage | Asks | Ends the action by |
|---|---|---|
| `preCond` | what has to be so first: a list of conditions such as `touchObj` (the player can touch it) or `objHeld` (the player is holding it), which Hexad tries to bring about | a condition that cannot be met |
| `verify()` | does the command make sense for this object? | `illogical(msg)`: never; `illogicalNow(msg)`: not now; `implausible(msg)`: unlikely |
| `check()` | is it allowed now? | printing anything at all |
| `action()` | what happens | (it is done) |
| `report()` | what the player is told | |

`verify()` also helps Hexad work out which object the player meant. Of two things called `bell`, the one whose handler says RING makes sense is the one rung. So `verify()` is the place for what is true of the object; `check()` is for what is true of the moment.

`report()` is kept apart from `action()` because one command can act on several objects (TAKE ALL), and Hexad then gathers the reports.

## A thing's own handler

Every thing has handlers for every action, from its class. A thing changes what an action does to it by having a handler of its own, written with the role (`dobj` or `iobj`) and the action:

```hexicon
// The bell handles RING itself.
+ bell: Fixture 'bell; great bronze'
    "A great bronze bell<<if humming>>, still humming<<end>>. "
    humming = nil

    dobj Ring
    {
        verify() { }
        action() { startHumming(); }
        report() { "BONG. The note rolls out over the town. "; }
    }
```

(What `startHumming()` does is below, in [Things that happen later](#things-that-happen-later).)

The stages it gives replace those of its class; those it leaves out are its class's. An empty `verify()` says RING makes sense for the bell, whatever the class says.

### Checking first

A `check()` stops the action by saying why:

```hexicon
    dobj Pull
    {
        verify() { }
        check()
        {
            if (!isClean)
                "The lever is rusted solid. ";
        }
        action() { "A trapdoor opens in the floor, and the game is done. "; }
    }
```

```transcript
>pull lever
The lever is rusted solid.
```

### One action as another

A thing can handle one action as it handles another, with `as`: RING ROPE does what PULL ROPE does. And an action can be replaced by another altogether, with `replaceAction()`: pulling the rope rings the bell.

```hexicon
// Pulling the rope rings the bell.
+ rope: Fixture 'rope; bell'
    "A thick bell rope. "

    dobj Pull
    {
        verify() { }
        action() { replaceAction(Ring, bell); }
    }
    dobj Ring as Pull
;
```

```transcript
>ring rope
BONG.  The note rolls out over the town.

>pull rope
BONG.  The note rolls out over the town.
```

### The indirect object

In a two-object action, each object has a handler for its role. CLEAN LEVER WITH RAG asks the lever's `dobj CleanWith` and the rag's `iobj CleanWith`. By default nothing can be cleaned with, so the rag says it can:

```hexicon
    // Something to clean with: the rag accepts CLEAN ... WITH it.
    iobj CleanWith
    {
        verify() { }
    }
```

Both objects must accept: had the rag no handler of its own, CLEAN LEVER WITH RAG would be refused by the rag, whatever the lever said.

In any stage of a handler, the command's objects are `now.dobj` and `now.iobj`. So a thing's handler for one role can ask what is in the other. The niche takes only the rag:

```hexicon
// A niche for the rag, and nothing else: now.dobj is the thing being put
// in it.
+ niche: Container, Fixture 'niche; stone'
    "A niche in the stone wall. "

    iobj PutIn
    {
        check()
        {
            if (now.dobj != rag)
                "The niche is for the rag. ";
        }
    }
;
```

```transcript
>put feather in niche
The niche is for the rag.

>put rag in niche
You put the oily rag in the niche.
```

### Using what Hexad already does

Many actions come with properties that set up their usual cases, so that a handler is not needed at all. CLEAN uses `needsCleaning`, `mustBeCleanedWith`, and `isClean`, and calls `makeCleaned()` to do the cleaning:

```hexicon
// A lever that works only once it is clean.
+ lever: Fixture 'lever; rusty'
    "An iron lever, <<if isClean>>bright and clean<<else>>red with rust<<end>>. "
    needsCleaning = true
    mustBeCleanedWith = rag

    makeCleaned(stat)
    {
        inherited(stat);
        "The rust comes away on the rag. ";
    }
```

With `mustBeCleanedWith` set, the player need not say what to clean it with:

```transcript
>clean lever
(with the oily rag)
The rust comes away on the rag.
```

PUSH has `isPushable`, LOCK `lockability`, EAT `isEdible`, and so on: Hexad's own handlers, in `thing.hxn` in its sources, say which each uses. Some of these only decide whether the action makes sense, and leave what happens to the game. POUR is one: a thing that can be poured has `isPourable` (and `fluidName`, for an oilcan that pours oil: `'the oil'`), and a thing that may have something poured onto it has `allowPourOntoMe`; but pouring itself changes nothing, so the thing poured onto has an `iobj PourOnto` with an `action()` that says what happens. ATTACH is another; see [Containers, Surfaces, Doors, and Keys](containers-and-doors.md#things-attached-to-things).

## Things that happen later

Something that is to happen some turns from now is a **fuse**: `new Fuse(obj, &method, turns)` calls the object's method after that many turns. Ringing the bell starts it humming, and two turns later it stops. Rung again, it starts over: `removeEvent()` puts out the fuse that was burning.

```hexicon
    // Ringing it starts it humming, and two turns later it stops: a Fuse.
    hum = nil

    startHumming()
    {
        humming = true;
        if (hum != nil)
            hum.removeEvent();
        hum = new Fuse(self, &stopHumming, 2);
    }

    stopHumming()
    {
        humming = nil;
        hum = nil;
        "The bell's hum dies away. ";
    }
;
```

```transcript
>sing
Your voice echoes round the stone walls.

The bell’s hum dies away.
```

A **daemon** is the same, every so many turns rather than once: `new Daemon(obj, &method, 1)` calls the method every turn, until its `removeEvent()`. Either runs whether or not the player is there to see, so a method that prints should first ask whether they are (`me.isIn(belfry)`).

## New actions

A game can add actions of its own. An action is declared with the objects it takes, and the class it is:

```hexicon
// A new action, RING, which takes one object.
action Ring(dobj): TAction;
```

`TAction` is an action with a direct object, `TIAction` one with both, `IAction` one with none. A game's action, or a rule of its grammar, with the name of one of Hexad's takes the place of Hexad's, and the build says so with a warning; so a game written before Hexad had an action keeps its own.

Then its **grammar**, the words that mean it:

```hexicon
// The words that mean it.
verb Ring 'ring' singleDobj:
    verbPhrase = 'ring/ringing (what)'
    missingQ = 'what do you want to ring'
;
```

A `verb` declaration names the action, then says what the player types, up to a colon: words in quotes, alternatives between `|` in parentheses (`('push' | 'press')`), and where the objects go: `singleDobj` for one direct object, `multiDobj` for one or several, `singleIobj` for the indirect object. After the colon come the rule's properties. `verbPhrase` is how Hexad speaks of the action, and `missingQ` what it asks when the object is left out:

```transcript
>ring
What do you want to ring?
```

Last, what it does to things that have no handler of their own, which is to every thing in the game until it says otherwise. `modify Thing` gives Thing a handler for it:

```hexicon
// What RING does to anything that has no handler of its own.
modify Thing
    dobj Ring
    {
        verify() { illogical(cannotRingMsg); }
    }
    cannotRingMsg = '{The subj dobj} {is} not something you can ring. '
;
```

`{The subj dobj}` and `{is}` are **message parameters**: Hexad puts the direct object's name, and the verb in the right form for it.

```transcript
>ring rag
The oily rag is not something you can ring.
```

### An action with no object

An `IAction` has no handlers, since it has no objects. What it does is its `execAction()` method:

```hexicon
// A new action, SING, which takes none.
action Sing: IAction
    execAction(cmd)
    {
        "Your voice echoes round the stone walls. ";
    }
;

verb Sing 'sing':
    verbPhrase = 'sing/singing'
;
```

### More words for an action

An action can have more than one rule. Each rule has a name of its own, and `for` says which action it is for when the name is not the action's:

```hexicon
// Another rule for RING.
verb Toll for Ring 'toll' singleDobj:
    verbPhrase = 'toll/tolling (what)'
    missingQ = 'what do you want to toll'
;
```

```transcript
>toll bell
BONG.  The note rolls out over the town.
```

Hexad's own rules are in its English grammar (`english/grammar.hxn` among its sources), each named for its action. `modify verb` changes one. The words given replace the rule's words, and the rest of the rule is kept. `replace verb` replaces the whole rule:

```hexicon
// Hexad's rule for JUMP, with another word.
modify verb Jump ('jump' | 'leap'): ;
```
