# Containers, Surfaces, Doors, and Keys

Things go in, on, under, and behind other things, and some of those open, close, and lock. This page is about all of them, in Hexad. Its example, *The Study*, is in [examples/study](examples/study); each piece of code here is part of it.

## Where a thing is

A thing's `location` is the object it is in, on, under, or behind. A `+` in front of an object's definition gives it the location of the object defined before it with one fewer `+`, so most things need no `location` written out. But it is the last object *without* a `+` that a single `+` refers to, so an object defined in the middle of a room's things, with no `+` of its own, takes the room's place for the rest of them. The study's key is hidden, and has no location of its own, so it is defined last, where it is in nobody's way:

```hexicon
// The key is hidden, so it has no location: it is defined here, after the
// objects with a +, which go in the object defined before them that has
// none.
smallKey: Key 'small key'
    "A small key. "
;
```

Had it come before the wardrobe and the door, they would have been in the key, and the player would have been carrying them.

## Containers and surfaces

How a thing holds others depends on its class:

| Class | Things go | PUT ... | It opens |
|---|---|---|---|
| `Container` | in it | PUT IN | no: it is always open |
| `OpenableContainer` | in it | PUT IN | yes, and starts closed |
| `LockableContainer` | in it | PUT IN | yes, and locks without a key; starts locked |
| `KeyedContainer` | in it | PUT IN | yes, and locks with a key; starts locked |
| `Surface` | on it | PUT ON | no |
| `Underside` | under it | PUT UNDER | no |
| `RearContainer` | behind it | PUT BEHIND | no |
| `Booth` | in it, the player too | GET IN | no |
| `Platform` | on it, the player too | GET ON | no |

A `Booth` or `Platform` that is somewhere to hide, a wardrobe or a locker, says `isHidingPlace = true`: HIDE on its own then gets in it, as HIDE IN THE WARDROBE does anywhere.

A room's exit can lead into one: with `in = wardrobe`, IN is GET IN THE WARDROBE. A connector's `destination` can be one too, in another room, and the traveler arrives in it.

Each of these can be taken, like any `Thing`. One that cannot is given `Fixture` as well, as its second class:

```hexicon
// Something to put things on.
+ shelf: Surface, Fixture 'shelf'
    "A plain shelf. "
;

++ vase: Thing 'vase; blue'
    "A blue vase. "
;
```

The vase is on the shelf, and the room says so:

```transcript
You can see a wicker basket and a puzzle box here.

On the shelf you see a vase.

On the writing desk you see an inkwell.

In the glass cabinet you see a medal.
```

What is inside a closed container cannot be seen, unless it is `isTransparent`, like the glass cabinet. Examining something that can be opened says when it is closed ("It's closed."); a thing whose own description already says so sets `closedStatusReportable = nil`. Opening one shows what it holds:

```transcript
>open cabinet
Opening the glass cabinet reveals a medal.
```

`bulk` and `bulkCapacity` set how much fits: a thing's `bulk` is its size, in whatever units the game likes, and a container's `bulkCapacity` the most it can hold. Both are unlimited by default, in effect: things have no bulk, and containers a capacity of 10,000.

## One thing with several parts

A writing desk has a top and a drawer: things go on it and in it. One object can be both, through parts that stand in for it:

```hexicon
// One thing with a top and a drawer: two parts, each a SubComponent.
// The player may call it the drawer, so that is one of its nouns.
+ desk: Fixture 'writing desk; ; drawer'
    "An old writing desk, with a drawer. "

    remapOn: SubComponent { }
    remapIn: SubComponent { isOpenable = true }
;
```

`remapOn` is the part that PUT ON and the like go to, and `remapIn` the part that PUT IN, OPEN, CLOSE, LOOK IN, and LOCK go to. There are `remapUnder` and `remapBehind` too. Each is an object written in place, between braces: a `SubComponent`, which works out from the property it is in how it holds things. The drawer opens, so OPEN DESK opens it. The parts have no vocabulary of their own: the player says `desk`, and what a player may call a part, `drawer` here, goes among the desk's nouns, so that OPEN DRAWER and CLOSE DRAWER work too:

```transcript
>close drawer
Done.
```

A thing starts in a part when its `subLocation` names it:

```hexicon
++ inkwell: Thing 'inkwell'
    "A brass inkwell. "
    subLocation = &remapOn
;

++ pen: Thing 'pen'
    "A steel pen. "
    subLocation = &remapIn
;
```

`&remapOn` is the property itself, rather than its value. The inkwell is on the desk, and the pen in its drawer:

```transcript
>open desk
Opening the writing desk reveals a pen.
```

## Hidden under, behind, or in

A thing can be hidden under, behind, or in another, to be found by looking there. It is listed in the other's `hiddenUnder`, `hiddenBehind`, or `hiddenIn`, and has no location until it is found:

```hexicon
// Something hidden under the rug, found by looking.
+ rug: Fixture 'rug'
    "A worn rug. "
    hiddenUnder = [smallKey]
;
```

```transcript
>look under rug
Under the rug you find a small key, which you take.
```

(See [Things and Rooms](things-and-rooms.md) for the other way to hide a thing, `isHidden`.)

## Locks and keys

A thing that locks has a `lockability`:

| `lockability` | Locked and unlocked |
|---|---|
| `notLockable` | never: it has no lock |
| `lockableWithoutKey` | by LOCK and UNLOCK alone, like a bolt |
| `lockableWithKey` | by LOCK and UNLOCK with a key |
| `indirectLockable` | not by command at all, but by something the game does: a lever, a combination. LOCK and UNLOCK are told it uses some other kind of mechanism |

The lock can also be a rule rather than a state: `isLocked = (!keycard.isIn(now.actor))` is a door that opens only for whoever carries the keycard. A door's other side follows the rule.

A thing with a `keyList` is `lockableWithKey` unless it says otherwise, and anything lockable starts locked (`isLocked`). `LockableContainer` and `KeyedContainer` set these for a container:

```hexicon
// Locked with a key; seen into even when closed.
+ cabinet: KeyedContainer, Fixture 'glass cabinet'
    "A cabinet with glass doors. "
    isTransparent = true
    keyList = [smallKey]
;
```

```hexicon
// Locked and unlocked without a key.
+ puzzleBox: LockableContainer 'puzzle box; little'
    "A little puzzle box. "
;
```

```transcript
>open box
The puzzle box is locked.

>unlock box
Unlocked.
```

A `Key` needs nothing more: the things that list it in their `keyList` are the things it locks. Keys can go on a `Keyring`, and only keys can. A key on a keyring the player holds is at hand to lock and unlock with, without being taken off, and a key the player picks up while holding one goes onto it, unless the ring says `collectsKeys = nil`:

```hexicon
// Keys go on it, and a key on it is at hand to lock and unlock with.
+ keyring: Keyring 'keyring; steel; ring'
    "A steel keyring. "
;
```

```transcript
>put key on keyring
You put the small key on the keyring.

>i
You are carrying a keyring (on which is a small key).
```

```transcript
>lock cabinet
(with the small key)
Locked.
```

 A key can also list what it fits itself, in `actualLockList`; the things it looks as if it might fit, in `plausibleLockList`; and those the player knows it fits, in `knownLockList`, which fills as they use it. When the player says UNLOCK without saying with what, Hexad uses a key they are holding: one known to fit, if they have one, or else one that might. A thing's `keyList` puts it among its keys' plausible locks.

## Doors

A door is two objects, one in each room, each the `otherSide` of the other, as in [Getting Started](getting-started.md):

```hexicon
// A door that opens and closes, but does not lock.
+ studyDoor: Door 'door'
    "A plain door. "
    otherSide = hallDoor
;
```

```hexicon
+ hallDoor: Door 'door'
    "A plain door. "
    otherSide = studyDoor
;
```

A door is opened and closed, and locked with a `keyList` or a `lockability` of its own, like any other thing that does; and whatever happens to one side happens to the other. It starts closed, unless it says `isOpen = true`. A player who walks into a closed door, if it is not locked, opens it first:

```transcript
>north
(first opening the door)

Hall
A cold hall, with a brass hook on the wall.  The study is to the south.
```

## Things attached to things

ATTACH and DETACH need things that can be attached, and things they can be attached to. A `SimpleAttachable` is either; its `allowableAttachments` lists what can be attached to it:

```hexicon
// Something to attach things to, and something to attach.
+ hook: SimpleAttachable, Fixture 'brass hook'
    "A brass hook. "
    allowableAttachments = [picture]
;

+ picture: SimpleAttachable 'small picture; framed'
    "A small framed picture of the sea. "
;
```

A thing attached is in the thing it is attached to, and its `attachedTo` says which. A part attached for good, such as a label on a box, is an `AttachableComponent`: it starts attached to its location, moves with it, and cannot be detached or taken. Examining a thing says what is attached to it:

```transcript
>attach picture to hook
You attach the small picture to the brass hook.

>x hook
A brass hook.  Attached to it is a small picture.
```
