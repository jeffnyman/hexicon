# Things and Rooms

Everything the player can see, touch, or carry is a **thing**, and every place they can be is a **room**. This page is about both, in Hexad. Its example, *The Attic*, is in [examples/attic](examples/attic); each piece of code here is part of it.

## Things

A thing is an object of the class `Thing`, or of a class built on it:

```hexicon
+ letter: Thing 'letter'
    "A letter, addressed to nobody. "
    initSpecialDesc = "A letter lies on the floor. "
    readDesc = "It is blank. "
;
```

Its **vocabulary** comes first, in single quotes: its name, then after a semicolon its adjectives, then its other nouns, then the pronouns it is called by (see [Getting Started](getting-started.md)). Its **description** comes next, in double quotes: what EXAMINE shows. Both are given by the class's template, so they need no names.

### Can it be taken?

A `Thing` can be picked up and carried. These classes cannot be:

| Class | What it is | Trying to take it |
|---|---|---|
| `Fixture` | Fixed in place: furniture, a portrait on a wall | `The portrait is fixed in place.` |
| `Heavy` | A fixture too heavy to move | `The trunk is too heavy to move.` |
| `Decoration` | Scenery, only to be looked at: every action but EXAMINE and GO TO gets `The window is not important.` | as for any other action |
| `Distant` | A decoration out of reach: every action but EXAMINE gets `The church spire is too far away.` (GO TO too, unless it has a `destination`) | as for any other action |
| `Unthing` | Something not there at all, which the player may try to refer to: every action gets its `notImportantMsg` | as for any other action |

```hexicon
// Fixed in place, and so not listed: the room's description mentions it.
+ portrait: Fixture 'portrait; old oil; painting'
```

```hexicon
// Too far away to touch.
+ spire: Distant 'church spire; distant'
    "Through the window, a church spire rises over the roofs. "
;

// Not here at all.
+ ghost: Unthing 'ghost'
    notImportantMsg = 'There is no ghost. There never was. '
;
```

The message a fixture gives is its `cannotTakeMsg`, and a decoration's is its `notImportantMsg`; either can be given a value of its own. The actions a decoration does answer are its `decorationActions`.

### What a room's description lists

When the player looks around, a room's description is followed by its contents:

```transcript
Landing
A narrow landing at the top of the house, with a portrait on one wall and a
small window in the other.  A ladder leads up into the attic, and the stairs
lead down.

A letter lies on the floor.

A grandfather clock ticks in the corner.

You can see a candle, a sprig of lavender, an apple, and a scarf here.
```

- **Things that can be taken are listed**, together, in the last paragraph.
- **Fixtures, and the classes built on them, are not**, since a room's own description is the place to mention them, as the landing's mentions the portrait and the window. `isListed` decides it: `true` for a thing that can be taken, `nil` for one that cannot, and either can be given to any thing.
- **A `specialDesc` is a paragraph of its own**, shown whenever the thing is there, whether or not it is listed:

```hexicon
+ clock: Fixture 'grandfather clock'
    "A tall clock in a dark case. "
    specialDesc = "A grandfather clock ticks in the corner. "
    listenDesc = "Tick. Tock. "
;
```

- **An `initSpecialDesc` is a paragraph of its own until the thing is first moved**, and then it is listed with the rest. The letter lies on the floor until the player picks it up:

```transcript
>take letter
Taken.

>look
Landing
A narrow landing at the top of the house, with a portrait on one wall and a
small window in the other.  A ladder leads up into the attic, and the stairs
lead down.

A grandfather clock ticks in the corner.
```

### The senses

Besides its description, a thing can have something for each sense:

| Property | The command it answers |
|---|---|
| `readDesc` | READ |
| `smellDesc` | SMELL |
| `listenDesc` | LISTEN TO |
| `feelDesc` | FEEL, TOUCH |
| `tasteDesc` | TASTE |

A thing without one gets Hexad's own reply: `There is nothing to read on the apple`, `You smell nothing out of the ordinary`, and so on. A smell or a sound is also part of what SMELL or LISTEN on its own reports, unless the thing says otherwise (`isProminentSmell = nil`, `isProminentNoise = nil`):

```transcript
>listen
Tick.  Tock.
```

### Food and clothing

A `Food` can be eaten, and a `Wearable` worn:

```hexicon
+ apple: Food 'apple; red'
    "A red apple. "
    tasteDesc = "Sweet. "
;

+ scarf: Wearable 'scarf; woollen wool'
    "A long woollen scarf. "
;
```

They are `Thing`s with `isEdible` or `isWearable` set, which any thing can be given. The player need not pick them up first: Hexad does that for them.

```transcript
>eat apple
(first taking the apple)
You eat the apple.

>wear scarf
(first taking the scarf)
Okay, you’re now wearing the scarf.
```

### Hidden things

A thing with `isHidden = true` is there, but cannot be seen or referred to, until something calls its `discover()` method:

```hexicon
+ note: Thing 'folded note'
    "A note, folded small. "
    readDesc = "<q>Do not go up without a light.</q> "
    isHidden = true
;
```

Here the portrait finds it, the first time it is examined:

```hexicon
    "An old oil portrait of a stern woman<<if note.isHidden>>. Something
    is tucked behind the frame<<note.discover()>><<end>>. "
```

A description can have code in it, between `<<` and `>>`. `<<if ...>>` shows what follows only if the condition holds, up to `<<end>>`; and `<<note.discover()>>` runs the method, showing nothing. So the note is mentioned, and found, once:

```transcript
>x portrait
An old oil portrait of a stern woman.  Something is tucked behind the frame.

>take note
Taken.
```

## Rooms

A room is an object of the class `Room`. Its template takes its title, then its description:

```hexicon
// A room, with an exit that only says something.
landing: Room 'Landing'
    "A narrow landing at the top of the house, with a portrait on one wall
    and a small window in the other. A ladder leads up into the attic, and
    the stairs lead down. "
```

### Exits

An exit is a property named for its direction: `north`, `south`, `east`, `west`, `northeast`, `northwest`, `southeast`, `southwest`, `up`, `down`, `in`, and `out`, and on board ship `port`, `starboard`, `fore`, and `aft`. Its value says what happens when the player goes that way:

- **a room**, to go there;
- **a door** or another connector, to go through it to wherever it leads;
- **a string in double quotes**, to show it and go nowhere.

```hexicon
    up = attic
    down = "You have not finished up here yet. "
    cannotGoThatWayMsg = 'The walls of the landing close in on every side. '
```

A direction with no exit gets the room's `cannotGoThatWayMsg`, and Hexad adds the ways the player can go:

```transcript
>west
The walls of the landing close in on every side.  From here you could go down
or up.

>down
You have not finished up here yet.
```

Exits go one way. The landing's `up` reaches the attic, and the attic has a `down` of its own to come back.

### Light and dark

A room is lit, unless it says it is not:

```hexicon
attic: Room 'Attic'
    "Dust lies thick on the boards, and the roof slopes low. A ladder leads
    down. "

    down = landing
    isLit = nil
;
```

A dark room can be seen in only by a light in it. In the dark the player sees the room's `darkName` and `darkDesc`, which say so, and can do little but leave:

```transcript
>up
In the dark
It is pitch black; you can’t see a thing.
```

Any thing with `isLit = true` is a light. A thing with `isLightable = true` can be lit by the player, and put out:

```hexicon
+ candle: Thing 'candle'
    "A stub of candle. "
    isLightable = true
;
```

```transcript
>light candle
Done.

>up
Attic
Dust lies thick on the boards, and the roof slopes low.  A ladder leads down.
```

A thing with `visibleInDark = true` can be seen in the dark without lighting anything else, like the stars.
