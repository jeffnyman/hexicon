# Characters and Conversation

The people in a game, and what they say. Its example, *The Ferry*, is in [examples/ferry](examples/ferry); each piece of code here is part of it.

## Characters

A character is an `Actor`: a thing, with a vocabulary and a description, that can be talked to.

```hexicon
// The ferryman, and the two states he can be in.
+ ferryman: Actor 'ferryman; old; man him'
    "A lean old man in a patched coat. "
;
```

The fourth part of the vocabulary gives the pronouns he is called by: here `him`, so that ASK HIM ABOUT works. A character with a name of his own is given it with a capital, `'Ada; old; woman her'`: a name whose every word starts with a capital is a **proper name**, which Hexad uses without "the" ("Ada won't let you pick her up").

### States

A character is often in one of several **states**: waiting, asleep, following the player. Each is an `ActorState` inside the actor, and the one with `isInitState = true` is the one he starts in:

```hexicon
++ waitingState: ActorState
    isInitState = true
    specialDesc = "The ferryman leans on his pole by the water. "
    stateDesc = "He is waiting for a fare. "
;

++ paidState: ActorState, ShuffledEventList
    specialDesc = "The ferryman stands in his boat, ready to push off. "
    stateDesc = "He has been paid. "
```

The state says how he looks. Its `specialDesc` is his paragraph in the room's description, and its `stateDesc` is added to his own description when he is examined:

```transcript
>x ferryman
A lean old man in a patched coat.  He is waiting for a fare.
```

`setState()` moves him from one state to another; below, paying him does.

### What a character does on his own

A state can also be an event list (`ActorState, ShuffledEventList`, with an `eventList`): it shows the next of its messages, the small things he does while he waits. It would every turn; `eventPercent` makes it the given part of the turns:

```hexicon
    eventList = [
        'The ferryman spits into the river. ',
        'The ferryman looks at the sky. ',
        'The ferryman shifts his pole from one hand to the other. '
    ]
    eventPercent = 50
;
```

```transcript
>wait
Time passes.

The ferryman spits into the river.
```

 Hexad runs it only on a turn the player is not talking to him, and only while the player can see him, so that he does not fidget through a conversation, or out of sight. So a state's list is for what he is seen doing. What must happen whether or not anyone is watching, such as finishing a meal or walking off after a while, belongs elsewhere:
- in an `AgendaItem`, which he carries out whether the player can see him or not, though, like the list, not on a turn he is talked to;
- or in a `Daemon` or a `Fuse`, which runs every turn, or after so many, whatever else is going on.

## What characters say

What a character says is in **topic entries** inside him. Each answers one kind of command, about one thing:

```hexicon
// Greetings, and goodbyes. Every conversation starts with a greeting, so
// the first is said once, and the next one after.
++ HelloTopic, StopEventList
    [
        '<q>Crossing to the island?</q> he asks.<.reveal island> ',
        '<q>You again,</q> he says. '
    ]
;

++ ByeTopic
    "<q>Mind the mud,</q> he says. "
;
```

| Topic entry | Answers |
|---|---|
| `HelloTopic` | the start of a conversation: TALK TO, HELLO, or a first question |
| `ByeTopic` | its end: BYE, or walking away |
| `AskTopic`, `TellTopic`, `AskTellTopic` | ASK ABOUT, TELL ABOUT, or either |
| `AskForTopic` | ASK FOR |
| `GiveTopic`, `ShowTopic`, `GiveShowTopic` | GIVE TO, SHOW TO, or either |
| `QueryTopic` | questions: ASK WHO, WHAT, WHY, WHERE, WHEN, HOW, WHETHER, and IF, as its `qType` says |
| `SayTopic` | SAY |
| `YesTopic`, `NoTopic` | YES and NO |
| `CommandTopic` | an order given to him: FERRYMAN, ROW |
| `DefaultAnyTopic` | anything no other entry answers |

There are defaults for each kind too (`DefaultAskTopic`, `DefaultGiveTopic`, and so on), for anything of that kind no other entry answers.

### What a topic is about

An entry's `@` names what it is about. That can be a thing in the game, such as the map, or a **topic**: something that can be talked about but is not a thing in the game.

```hexicon
tRiver: Topic 'river';
tIsland: Topic 'island';
```

```hexicon
++ ShowTopic @map
    "He squints at it. <q>Old,</q> he says, <q>but right.</q> "
;
```

The player can only talk about what they know of: a thing they have seen, or a topic that is `familiar`. A `Topic` is, unless it says `familiar = nil`, and a thing can be made so before it is seen, as the lamp is in [Getting Started](getting-started.md). A thing a character mentions is not known just because he mentions it: what he says can make it so with `<.known name>`, `name` being the thing's name in the source. The ferryman's chapel is on the island, and is not in the game at all, but once he has spoken of it the player can ask about it:

```hexicon
    "<q>Nobody lives there now,</q> he says, <q>but for the old chapel.
    It's a penny to cross.</q><.known chapel> "
    isActive = now.revealed('island')
;

// Something he mentions, which the player has not seen.
++ AskTopic @chapel
    "<q>Roof's gone,</q> he says. <q>Bell's still there.</q> "
;
```

```transcript
>ask ferryman about chapel
“Roof’s gone,” he says.  “Bell’s still there.”
```

### Starting a conversation

A conversation starts with a greeting, every time: after BYE, or walking away, the next question starts another. That is why the ferryman's `HelloTopic` is a `StopEventList` (see [Several responses](#several-responses)): his first words would be wrong the second time. If the player asks or tells something before saying hello, the character's `HelloTopic` comes first, and then the answer:

```transcript
>ask ferryman about island
“Crossing to the island?” he asks.

He only grunts.
```

### Topics that come later

Here the answer is the `DefaultAnyTopic`'s, because the entry about the island is not yet active:

```hexicon
// Only once the island has been mentioned.
++ AskTellTopic @tIsland
```

An entry is only used while its `isActive` is true. `<.reveal island>` in what the ferryman says on being greeted makes `now.revealed('island')` true, from then on, so the question works the second time it is asked:

```transcript
>ask ferryman about island
“Nobody lives there now,” he says, “but for the old chapel.  It’s a penny to
cross.”
```

`<.reveal key>` can be in anything the game prints, and `now.reveal('key')` does the same from code.

### Several responses

An entry can say something different each time. It is then an event list as well as a topic entry, and its responses are a list:

```hexicon
// One response after another, and then the last again.
++ AskTopic, StopEventList @tRiver
    [
        '<q>Deep and cold,</q> he says. ',
        '<q>Deep, I said. And cold.</q> ',
        '<q>Ask me about something else.</q> '
    ]
;
```

A `StopEventList` goes through its list and stays on the last. A `CyclicEventList` starts again from the top, and a `ShuffledEventList` picks in a random order.

```transcript
>ask him about river
“Deep and cold,” he says.

>ask him about river
“Deep, I said.  And cold.”

>ask him about river
“Ask me about something else.”

>ask him about river
“Ask me about something else.”
```

### Responses that do things

A response can be a method, to do more than talk. Paying the ferryman takes the penny and changes his state:

```hexicon
++ GiveTopic @penny
    topicResponse()
    {
        "He bites the penny, and nods. ";
        penny.moveInto(nil);
        ferryman.setState(paidState);
    }
;
```

`moveInto(nil)` takes the penny out of the game.

### Topics in a state

Topic entries inside a state are used before the actor's own, while he is in that state. Once paid, the ferryman answers about the island differently:

```hexicon
// A topic in a state is used before those in the actor, while he is in it.
+++ AskTellTopic @tIsland
    "<q>Sit down, and we'll be off,</q> he says. "
;
```

```transcript
>give penny to ferryman
He bites the penny, and nods.

>x ferryman
A lean old man in a patched coat.  He has been paid.
```

```transcript
>ask him about island
“Sit down, and we’ll be off,” he says.
```
