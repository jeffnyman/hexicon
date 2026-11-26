# Telling the Story

A game is told, by default, in the second person and the present tense: "You pull the cord." It can be told in the first person or the third, and in the past, and can change while it is played. Hexad's own messages follow by themselves. A game's own text follows only where it is written to, and this page is about how. Its example, *The Sitting Room*, is in [examples/telling](examples/telling); each piece of code here is part of it.

## Who tells it, and when

The player's `person` is 1, 2, or 3: "I pull the cord", "you pull the cord", or, in the third person, the player's name, "Ada pulls the cord". The player is given a name for the third person in their vocabulary, and the pronoun the story calls them by:

```hexicon
// The player, who has a name of her own for the third person.
+ me: Player 'Ada; ; woman; her'
    "{I} {am} in a borrowed dressing gown. "
    isHer = true
;
```

The tense is `gameMain.usePastTense`: `nil`, the present, or `true`, the past. Either can be changed at any time. A change of person changes the player's `name` with it, since the other forms (me, my, myself) are made from the name:

```hexicon
// How the story is told, which takes no turn.
tellingIn: object
    setPerson(n)
    {
        me.person = n;
        me.name = (n == 1 ? 'I' : n == 2 ? 'you' : 'Ada');
        "(Now in the <<['first', 'second', 'third'][n]>> person.) ";
    }
;
```

The commands that call it are `SystemAction`s, which take no turn.

## The game's own text

Text written plainly stays as it is written. For text to follow the story, the words that change are written as **message parameters**, in braces:

| Parameter | Stands for |
|---|---|
| `{I}`, `{me}`, `{my}`, `{mine}`, `{myself}` | the player: I, you, Ada; me, you, Ada; my, your, Ada's; and so on. Capitalized, `{I}` begins a sentence |
| a verb by name: `{am}`, `{is}`, `{can}`, `{have}`, `{pull}`, `{rings}`, `{hangs}` | the verb, agreeing with the subject before it and in the story's tense: "I am", "Ada is", "you were"; "Ada pulls", "I pulled" |
| a verb with its endings: `stroke{s/d}`, `pull{s/ed}`, `reach{es/ed}`, `carr{ies/ied}` | the same, for a regular verb, written out |
| `{dummy}` before a verb | a subject that is no character: "the window `{dummy}{is}` to the east", "was" in the past |
| `{plural}` before a verb | the same, plural: "the curtains `{plural}{are}` drawn" |

A verb can be named whether it is regular (`{pull}`, `{opens}`) or irregular (`{hangs}`, `{rings}`, `{comes}`); Hexad knows the irregular ones, and makes a regular one's forms by rule. Written with its endings instead, an irregular verb comes out wrong: `hang{s/ed}` gives "hanged".

```hexicon
// Text of the game's own, written so that it follows the person and the
// tense: every verb in braces.
sittingRoom: Room 'Sitting Room'
    "A low room, with a fire in the grate. The window {dummy}{is} to the
    east, and the curtains {plural}{are} drawn. A bell cord {dummy}{hangs}
    by the fireplace. "
;
```

```transcript
>look
Sitting Room
A low room, with a fire in the grate.  The window was to the east, and the
curtains were drawn.  A bell cord hung by the fireplace.

A grey cat was asleep by the fire.
```

Every piece of a game's text needs it, the `specialDesc` of the cat as much as the room's description; one written plainly stays in the present while the rest has gone into the past.

## Other characters, and things

For anyone or anything else, the text is given to `dmsg()` with the objects it names, which the parameters refer to by number: `{the 1}`, `{the subj 1}` (as the subject of the verb after it), `{he 1}`, `{him 1}` (or `{it 1}`), `{his 1}`:

```hexicon
    dobj Feel
    {
        verify() { }
        action()
        {
            dmsg('{I} stroke{s/d} {the 1}, and {he 1} {opens} one eye at
                {me}. ', cat);
        }
    }
```

```transcript
>touch cat
Ada stroked the cat, and he opened one eye at Ada.
```

`bmsg()` is the same, but gives the text back, as a string, instead of showing it.

## When something comes out in braces

A parameter Hexad does not know, such as `{yours 1}` written for `{his 1}`, is shown as it is written, braces and all, so that it can be found and put right. A word on its own in braces is taken for a verb, so a misspelled one (`{teh}`) is taken for one too; a transcript of each tense and person, read through once, shows both.
