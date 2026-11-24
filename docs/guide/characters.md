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

The fourth part of the vocabulary gives the pronouns he is called by: here `him`, so that ASK HIM ABOUT works.

### States

A character is often in one of several **states**: waiting, asleep, following the player. Each is an `ActorState` inside the actor, and the one with `isInitState = true` is the one he starts in:

```hexicon
++ waitingState: ActorState
    isInitState = true
    specialDesc = "The ferryman leans on his pole by the water. "
    stateDesc = "He is waiting for a fare. "
;

++ paidState: ActorState
    specialDesc = "The ferryman stands in his boat, ready to push off. "
    stateDesc = "He has been paid. "
;
```

The state says how he looks. Its `specialDesc` is his paragraph in the room's description, and its `stateDesc` is added to his own description when he is examined:

```transcript
>x ferryman
A lean old man in a patched coat.  He is waiting for a fare.
```

`setState()` moves him from one state to another; below, paying him does.

## What characters say

What a character says is in **topic entries** inside him. Each answers one kind of command, about one thing:

```hexicon
// Greetings, and goodbyes.
++ HelloTopic
    "<q>Crossing to the island?</q> he asks.<.reveal island> "
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

The player can only talk about what they know of: a thing they have seen, or a topic that is `familiar`. A `Topic` is, unless it says `familiar = nil`, and a thing can be made so before it is seen, as the lamp is in [Getting Started](getting-started.md).

### Starting a conversation

A conversation starts with a greeting. If the player asks or tells something before saying hello, the character's `HelloTopic` comes first, and then the answer:

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
    "<q>Nobody lives there now,</q> he says. <q>It's a penny to cross.</q> "
    isActive = gRevealed('island')
;
```

An entry is only used while its `isActive` is true. `<.reveal island>` in what the ferryman says on being greeted makes `gRevealed('island')` true, from then on, so the question works the second time it is asked:

```transcript
>ask ferryman about island
“Nobody lives there now,” he says.  “It’s a penny to cross.”
```

`<.reveal key>` can be in anything the game prints, and `gReveal('key')` does the same from code.

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
