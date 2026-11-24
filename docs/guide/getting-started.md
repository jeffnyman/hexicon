# Getting Started with Hexicon

Hexicon is a system for writing interactive fiction: games the player reads, and plays by typing commands such as `open the box` or `go north`. It has three parts:

- **the language** a game is written in, in `.hxn` files;
- **Hexad**, the library every game is built on: rooms, things, doors, characters, the parser that understands what the player types, and all the messages a game prints;
- **the `hexicon` program**, which builds a game from its sources into a single `.hxg` file, and plays it.

This guide takes you from nothing to a small finished game, *The Lighthouse*. Its full source is in [examples/lighthouse](examples/lighthouse).

## Getting Hexicon

Hexicon is written in Rust. To build it, install Rust (from [rustup.rs](https://rustup.rs)), and then, in Hexicon's source folder:

```
cargo build --release
```

That makes one program, `hexicon` (`hexicon.exe` on Windows), in `target/release`. It is all you need: Hexad and everything else a game is built on are inside it. Copy it wherever you like, or add its folder to your `PATH`.

## Your first game

`hexicon new` starts a game in a new folder:

```
hexicon new my-game
cd my-game
hexicon build
hexicon run my-game.hxg
```

The folder has two files:

- `hexicon.toml`, the game's **manifest**: its name, its sources, and the libraries it is built on.
- `start.hxn`, its first **source**: a room, a note to read, and the player.

`hexicon build` reads the manifest and writes the game, `my-game.hxg`. `hexicon run` plays it. Type `x note`, then `take note`, then `quit`. That is a whole game, if not a long one.

From here on, the way to work is to change `start.hxn`, build, and play again.

## How a game is written

A game is a set of **objects**: rooms, things in them, the player. Each is written as a name, a colon, the kind of thing it is (its **class**), and then what makes it different from others of its kind, ending with a semicolon:

```hexicon
shore: Room 'Rocky Shore'
    "Waves break on the rocks behind you. The lighthouse rises to the north,
    its door set into the stone. "

    north = lighthouseDoor
;
```

`shore` is the object's name, which the rest of the source refers to it by. `Room` is its class. Then come its **properties**. The first two have no names: a class says what such values mean, in a **template**. For a `Room`, the first string in single quotes is the room's title, and the one in double quotes is what the player reads on entering it. `north = lighthouseDoor` is a property with a name: going north from the shore leads through the lighthouse door.

A string in double quotes is text the game shows. A string in single quotes is text the game keeps, such as a name.

## The Lighthouse, step by step

### What the game is

Every game says what it is called, and how it begins:

```hexicon
import hexad;

// What the game is called, and who wrote it.
versionInfo: GameID
    IFID = '5C2A6E7B-1D4F-4A83-9E21-7B3C0F8D2A64'
    name = 'The Lighthouse'
    byline = 'by the Hexicon guide'
    version = '1'
;
```

`import hexad;` makes Hexad's classes and templates available to this file. Every source of a game starts with it.

`versionInfo` describes the game. Its `IFID` is a number that identifies this game and no other, in every version of it: `hexicon new` makes a new one for each game, so keep the one you are given.

```hexicon
gameMain: GameMainDef
    initialPlayerChar = me
    maxScore = 2

    showIntro()
    {
        "The storm has been building all afternoon, and the lighthouse has
        not been lit. Somebody has to do it.\b";
    }
;
```

`gameMain` says who the player is (`me`, below), what the best score is, and what to show before the first room. `showIntro()` is a **method**: a property whose value is code, run when the game asks for it. `\b` leaves a blank line.

### Rooms and exits

The shore is above. The lighthouse has two more rooms:

```hexicon
lighthouseBase: Room 'Foot of the Lighthouse'
    "A spiral stair climbs into the dark. The door leads back out, to the
    south. "

    south = insideDoor
    up = lampRoom
;
```

```hexicon
lampRoom: Room 'Lamp Room'
    "Glass on every side, and the sea black beyond it. The stair leads back
    down. "

    down = lighthouseBase
;
```

An exit names where it leads: a room (`up = lampRoom`), or a door, which leads to wherever its other side is. Exits only go one way, so each room names its own: going up from the foot of the lighthouse reaches the lamp room, and the lamp room says that down leads back.

### Things, and things in things

```hexicon
+ tinBox: OpenableContainer 'tin box; battered'
    "A battered tin box, wedged between two rocks. "
;

++ brassKey: Key 'small brass key'
    "A small brass key, green with age. "
;
```

A `+` in front of an object puts it in the object defined before it with one fewer: the box is on the shore, and with `++` the key is in the box. An `OpenableContainer` is something that holds things and opens and closes; it starts closed, so the key is out of sight until the player opens the box.

The first string is the thing's **vocabulary**, the words the player can call it by. Before a semicolon is its name (`tin box`); after it, adjectives (`battered`). A third part, after another semicolon, can give more nouns, and a fourth the pronouns it is called by:

```hexicon
+ keeper: Actor 'old keeper; grey-bearded; man him'
```

The keeper can be called `old keeper`, `grey-bearded keeper`, or `man`, and is `him`.

### A locked door

A door is seen from both sides, so it is two objects, one in each room, each the `otherSide` of the other:

```hexicon
+ lighthouseDoor: Door 'door; heavy wooden'
    "A heavy wooden door, studded with iron. "
    otherSide = insideDoor
    keyList = [brassKey]
;
```

```hexicon
+ insideDoor: Door 'door; heavy wooden'
    "The door you came in by. "
    otherSide = lighthouseDoor
;
```

`keyList` names the keys that fit. A door with keys starts out locked; unlocking one side unlocks the other, since they are the same door. `[brassKey]` is a **list**: a door could have several keys.

### A character to talk to

```hexicon
+ keeper: Actor 'old keeper; grey-bearded; man him'
    "The keeper is old and grey-bearded, and he is shivering. "
;

++ HelloTopic
    "<q>Thank goodness,</q> the keeper says. <q>My legs won't take the
    stairs tonight.</q> "
;

++ AskTopic @lamp
    "<q>It's at the top of the stair,</q> he says. <q>Just switch it on.</q> "
;

++ AskTopic @tStorm
    "<q>Worst I've seen,</q> he says. <q>There'll be ships out there.</q> "
;

++ DefaultAnyTopic
    "The keeper only shakes his head. "
;

tStorm: Topic 'storm';
```

An `Actor` is a character. What the keeper says is in **topic entries** inside him (`++`). A `HelloTopic` answers `talk to keeper`; an `AskTopic` answers `ask keeper about` whatever its `@` names; a `DefaultAnyTopic` answers anything else. `<q>` and `</q>` are quotation marks, which come out curled.

A topic can be a thing in the game (`@lamp`), or a `Topic`, which is something that can be talked about but is not a thing (`tStorm`, the storm). The player can only ask about things they know of, and they have not seen the lamp when they meet the keeper. So the lamp says it is known from the start:

```hexicon
    // Everyone knows of the lamp, so the player can ask about it before
    // seeing it.
    familiar = true
```

### Winning

The game ends when the player lights the lamp:

```hexicon
+ lamp: Fixture 'great lamp; brass'
    "A great brass lamp, cold and dark. "
    isSwitchable = true
```

A `Fixture` cannot be taken. `isSwitchable = true` lets the player switch it on and off. What happens when they do is in its `makeOn()` method:

```hexicon
    makeOn(stat)
    {
        inherited(stat);
        if (stat)
        {
            "The lamp flares, and its beam swings out over the water. ";
            litLamp.awardPointsOnce();
            finishGameMsg(ftVictory, [finishOptionUndo]);
        }
    }
```

`makeOn(stat)` is what Hexad calls to switch the lamp on (`stat` is `true`) or off (`nil`, which is Hexicon's false). The lamp's own `makeOn()` replaces the one every switchable thing has, so it starts by calling that one, `inherited(stat)`, to do the switching. Then, if the lamp is now on, it says so, awards a point, and ends the game as a win, offering UNDO among the choices.

Points come from **achievements**:

```hexicon
openedDoor: Achievement +1 "unlocking the lighthouse";
litLamp: Achievement +1 "lighting the lamp";
```

Each is worth a point, and `FULL SCORE` lists those the player has. The door's point is awarded when it is unlocked. Rather than write the door's `makeLocked()` method into the door, the game adds it with `modify`, which changes an object defined elsewhere:

```hexicon
modify lighthouseDoor
    makeLocked(stat)
    {
        inherited(stat);
        if (!stat)
            openedDoor.awardPointsOnce();
    }
;
```

`!stat` is "not `stat`": the door has just been unlocked. `awardPointsOnce()` awards the point only the first time, however often the door is locked and unlocked again.

Last, the player:

```hexicon
me: Player 'you'
    location = shore
;
```

`location` says where an object starts, which is what `+` says for the objects after a room.

### Playing it

Build the game in its folder and play it:

```
cd docs/guide/examples/lighthouse
hexicon build
hexicon run lighthouse.hxg
```

```transcript
>north
(first trying to open the door)
The door is locked.

>x box
A battered tin box, wedged between two rocks.

>open box
Opening the tin box reveals a small brass key.
```

Hexad does a good deal on its own: going north into a closed door tries to open it first, and opening the box shows what it held. Unlocking the door earns the first point:

```transcript
>unlock door with key
You unlock the door with the small brass key.

(Your score has just increased by one point.)
```

At the top of the stair:

```transcript
>turn on lamp
The lamp flares, and its beam swings out over the water.

(Your score has just increased by one point.)

*** YOU HAVE WON ***
```

## When something goes wrong

`hexicon build` says what it finds wrong, with the file and the line. Some problems are **warnings**: the game is built, but something in it is probably not what you meant. Hexicon looks out for one kind of mistake that is otherwise silent, a misspelled property:

```
start.hxn(94): warning: property "isSwitchabel" is given a value here, but nothing reads it - did you mean "isSwitchable"?
```

Without the warning, the lamp would simply not switch on, and the player would be told they cannot do that. The same goes for a handler for an action that does not exist:

```
start.hxn(132): warning: handler "dobj Opne" is never called - did you mean "dobj Open"?
```

Other problems are **errors**, and stop the build. The commonest is a missing semicolon at the end of an object. The next line then reads as part of the same object, and the message is about that line:

```
start.hxn(42): error: invalid lvalue - cannot apply "++" operator to expression
start.hxn(42): error: unterminated object definition
```

Line 42 is the key's `++ brassKey`, but the mistake is the line before it: the box has no `;`. When a message makes no sense where it points, look just above it.

## Where to go next

- Change *The Lighthouse*: add a room, a thing to find, something to ask the keeper about.
- Read more about Hexad: [Things and Rooms](things-and-rooms.md), and [Containers, Surfaces, Doors, and Keys](containers-and-doors.md).
- Hexad's classes are in its sources, which are part of Hexicon (`lib/hexad` in its source folder): `thing.hxn` for things, `travel.hxn` for rooms, exits, and doors, `actor.hxn` for characters and conversation.
