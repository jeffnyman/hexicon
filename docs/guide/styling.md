# Pictures, Sounds, and Style

A game can show pictures, play sounds, link words to commands, and have a look of its own: its colors, its type, how each part of its text is set. This page is about all of them. Its example, *The Gallery*, is in [examples/gallery](examples/gallery); each piece of code here is part of it.

## Two kinds of display

The same game plays on two kinds of display:

- **a display of text**: a terminal, with `hexicon run`. It shows words, and leaves out what it cannot show: a picture shows the words the game gives for it, if any, and a sound nothing.
- **a rich display**: Hexicon's player (`hexicon-player`), which is a window of its own, and the same page in a web browser. It shows the pictures, plays the sounds, makes links of the links, and shows the game in its own look.

A game is written once for both. What it writes for a rich display is markup, in its strings: HTML, as a web page has it, held to what Hexicon allows (see [What a game may write](#what-a-game-may-write)). A display of text takes the words from it.

`hexicon run --rich -I walk.txt game.hxg` plays a game as a rich display would, and writes what that display is given, as text: a way to see that a game says what you meant it to, without a window.

## Pictures

A picture is one of the game's resources: a file named in the manifest, which `hexicon build` puts in the game's file. JPEG, PNG, SVG, and WebP pictures can be shown:

```toml
# The picture and the sounds the game shows and plays.
resources = ["harbour.svg", "bell.svg", "bell-lit.svg", "chime.wav", "fountain.wav"]
```

The game names it as a `Picture`, with the file, and the words a display of text shows instead:

```hexicon
// The picture, among the game's resources, and what a display without
// pictures shows instead.
harbourPicture: Picture 'harbour.svg'
    'A small harbour at evening, under a setting sun.'
;
```

Its `show()` shows it, wherever the text it is called from has got to:

```hexicon
+ painting: Fixture 'painting; small oil; harbour picture'
    "A small oil painting of a harbour at evening.
    <<harbourPicture.show()>>"
;
```

On a display of text, the words:

```transcript
>x painting
A small oil painting of a harbour at evening.

A small harbour at evening, under a setting sun.
```

On a rich display, the picture, with the words for anyone who cannot see it:

```document
<div class="picture"><img src="harbour.svg" alt="A small harbour at evening, under a setting sun."></div>
```

A picture is centered on a line of its own, unless its `pictureAlign` is `'right'` or `'left'`, which puts it at that side with the text beside it. With `showsOnce = true` it is shown only the first time. A player who has turned pictures off sees the words instead, as on a display of text.

A picture can be a button. With a `pictureCommand`, clicking it types the command; with a `pictureHover`, another picture shows while the pointer is over it (or the button has the keyboard's focus), and with a `picturePressed`, while it is pressed:

```hexicon
// A picture that is a button: clicking it rings the bell, and it lights
// up while the pointer is over it.
bellPicture: Picture 'bell.svg'
    'It gleams, ready to be rung.'
    pictureHover = 'bell-lit.svg'
    pictureCommand = 'ring the bell'
    pictureAlign = 'right'
;
```

```document
A small brass bell on a stand. <a plain href="ring the bell"><img class="picture" src="bell.svg" align="right" hspace="30" hsrc="bell-lit.svg" alt="It gleams, ready to be rung."></a><br>
```

Its words should read well on a display of text, where they are all there is of it: there, the bell is "A small brass bell on a stand." and then "It gleams, ready to be rung."

## Sounds

A sound is a resource too, as a `Sound`: WAV, Ogg, and MP3. Its `play()` plays it. Sounds play in layers, and one in a layer does not stop one in another: `'foreground'` (the default) for a sound something makes, `'ambient'` for a place's sounds, `'bgambient'` for a place's sound behind those, and `'background'` for music. `soundRepeat` is how many times (or `'loop'`), `soundChance` makes it come now and then (a chance, as a percentage, each second), and `interrupts` stops what is playing in its layer so that it plays at once.

A sound can be quieter than it was made, with a `soundVolume` from 0 to 100, and can fade in as it starts and out as it ends, over a `soundFadeIn` and a `soundFadeOut` in seconds. A sound with a fade out fades out whenever it stops: at its end, when its layer is stopped, or when another sound interrupts it. `stopSoundLayer('ambient', 3)` stops a layer's sounds with a fade of its own, three seconds.

```hexicon
// A sound something makes, and a place's sound, which loops while the
// player is there, fading in as they arrive and out as they leave.
chimeSound: Sound 'chime.wav' interrupts = true;
fountainSound: Sound 'fountain.wav' soundLayer = 'ambient' soundRepeat = 'loop'
    soundFadeIn = 2 soundFadeOut = 2
;
```

The bell plays its sound when it is rung:

```hexicon
    dobj Ring
    {
        verify() { }
        action()
        {
            chimeSound.play();
            "The bell gives a clear, high note. ";
        }
    }
```

```document
<sound src="chime.wav" layer="foreground" interrupt>The bell gives a clear, high note. <br>
```

A room's `roomSounds` play while the player is there, and stop when the player leaves, however the player goes:

```hexicon
courtyard: Room 'Courtyard'
    "A courtyard of old stone, with a fountain in the middle. The gallery
    is to the north. "
    north = gallery
    in = gallery
    roomSounds = [fountainSound]
;
```

A display of text plays nothing, and the player of a rich display can turn sounds, or music, off.

## Links

A link is words the player can click on instead of typing a command: `aHref(command, words)`. The command is typed for them.

```hexicon
gallery: Room 'Gallery'
    "A quiet room with one painting on its wall, a brass plaque beside it,
    and a small <<aHref('ring the bell', 'bell')>> on a stand. A door leads
    out to the courtyard, to the south. "
```

A display of text shows the words. Hexad makes links of its own on a rich display: the exits in the status line, and the score.

## A game's look

A game's look is a style sheet, in CSS, as a web page has one: named in the manifest, and put in the game's file with its resources.

```toml
# Its look, on a display that shows it.
stylesheets = ["gallery.css"]
```

It styles the game's part of the window and nothing else: `body` is the game's page, and every other rule applies within it.

```css
/* The whole game: its page and its type. */
body {
  background: #f7f3ea;
  color: #2a2620;
  font-family: Georgia, "Times New Roman", serif;
}
```

### Hexad's text, by class

Hexad marks its own text with classes, so that a style sheet can set each part of it:

| Class | What it marks |
|---|---|
| `roomname` | the room's name, on a line of its own |
| `roomdesc` | the room's description |
| `roomcontents` | what the room's description lists |
| `inputline` | the prompt, and what the player types |
| `parser` | the parser's messages ("I don't know the word...") |
| `notification` | asides such as score changes |
| `assume` | what the parser assumed ("(the brass key)") |
| `tip` | tips the first time something comes up |
| `announceObj` | each object named, when a command does several |
| `statusroom`, `statusscore` | the status line's room and score |
| `picture` | a `Picture` |

```css
/* Hexad's own text, by its classes: the room's name, and its asides. */
.roomname {
  font-variant: small-caps;
  letter-spacing: 0.08em;
  color: #6b3e1e;
}
```

### Classes of the game's own

A game's text can have classes of its own, on the page's elements: a `<span>` within a line, made a block by the style sheet if it is to be one, inside a `<p>`, which a display of text shows as a paragraph:

```hexicon
// The plaque's words, in a class of the game's own: a line for a display
// of plain text, a brass plate on one that shows the game's look.
+ plaque: Fixture 'plaque; brass; plate'
    "A brass plaque.
    <p><span class=\"plaque\">Harbour at Evening<br>Oil on board, about
    1890</span>"
    readDesc = desc
;
```

```css
/* The game's own class: the brass plaque by the painting. */
.plaque {
  display: block;
  width: 18em;
  margin: 0.5em auto;
  padding: 0.6em 1em;
  border: 1px solid #a8823d;
  border-radius: 4px;
  background: #e9d8a6;
  color: #3a2a10;
  font-family: system-ui, sans-serif;
  font-size: 0.9em;
  text-align: center;
}
```

```transcript
>read plaque
A brass plaque.

Harbour at Evening
Oil on board, about 1890
```

A `<p>` in the text, rather than a `<p class>` with a closing `</p>`, is the way to a paragraph on both kinds of display: a display of text marks the start and the end of a paragraph each with a blank line, where a rich display needs only the start. A `<br>` that is there for a display of text alone can be hidden from a rich one in the style sheet (`.plaque + br { display: none }`).

A style sheet can say what changes with the size of the window, or with the player's light or dark setting, with `@media`:

```css
/* Narrow windows: the plaque the width of the text. */
@media (max-width: 30em) {
  .plaque {
    width: auto;
  }
}
```

## What a game may write

Hexicon holds a game's markup and style to a list, so that a game shows the same wherever it is played, and can do nothing in the window but show itself:

- **Elements**: those of HTML 3.2 (paragraphs, line breaks, headings, lists, tables, `<b>`, `<i>`, `<font>`, `<center>`, `<blockquote>`, `<img>`, image maps, links), and `<span>`, `<div>`, `<figure>`, and `<figcaption>`; with `class`, `style`, and `title` on any of them. TADS's own are understood too: `<sound>`, `<bq>`, `<nobr>`, `<tab>`, `<title>` (the window's title), `<aboutbox>` (what "About this game" shows), `<body>` (a window's colors and picture), and `<banner>` (a pane at a side of the window: `<banner id=notes align=right width=20%>...</banner>` puts what is in it there, in place of what that banner had, and `<banner remove id=notes>` takes it away). An `<img>` can have an `hsrc` and an `asrc`, the pictures for the pointer over it and for it pressed.
- **CSS**: colors and backgrounds; fonts; text alignment, decoration, indent, transform, and spacing; margins, padding, borders, and rounded corners; widths and heights; `display` (block, inline, inline-block, none, and flex); floats; list styles; the borders and spacing of tables; `@media` for the window's width, light and dark, contrast, and reduced motion; and `@font-face`, for a font among the game's resources.
- **Left out**: scripts, `onclick` and the like, forms, frames, and anything that names a file or an address outside the game's own resources. A link is always a command. What the list does not have is dropped before the display sees it, and a game's `!important` is taken out, since only the player may put their own choices first.

## The player's settings

On a rich display, the player has settings of their own: colors (the game's, light, dark, or high contrast), the font and its size, the width of the text, pictures, sounds, music, and links, and a style sheet of their own. The game's look comes first, unless the player says theirs does, as a reader of a web page can. Turning pictures, sounds, or links off tells the game as well, so that a `Picture` shows its words, and a `Sound` plays nothing.

A game's look is the start of how it is read, not the end. Colors with too little contrast between them, type too small to read, and words that are only in a picture shut some players out: a player can change them, but should not have to.
