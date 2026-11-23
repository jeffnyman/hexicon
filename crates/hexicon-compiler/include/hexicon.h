#charset "us-ascii"
#pragma once

/*
 *   Hexicon's system header: the VM's built-in functions under Hexicon's
 *   own names.
 *
 *   This header is built into the compiler. A program that includes it
 *   uses these names in place of t3.h's (t3GetGlobalSymbols becomes
 *   globalSymbols, and so on), and tadsSay is also print. The functions
 *   are the same ones: a function set is bound by its identifier
 *   ('t3vm/010006'), not by the names a header gives its functions, so
 *   the image is the same whichever names a program uses. This header
 *   and tads.h can both be included, in one source or in different
 *   ones.
 *
 *   The order of the functions in each set is their place in it, and
 *   must not change.
 */

#include <tadsgen.h>
#include <tadsio.h>
#include <systype.h>

/*
 *   The VM's own functions (t3.h).
 */
intrinsic 't3vm/010006'
{
    /* collect garbage now (t3RunGC) */
    collectGarbage();

    /* set the function or method that displays double-quoted strings and
     *   embedded expressions (t3SetSay) */
    setSay(val);

    /* the VM's version, as 0xMMmmrr (t3GetVMVsn) */
    vmVersion();

    /* the VM's identifier string (t3GetVMID) */
    vmId();

    /* the VM's banner string (t3GetVMBanner) */
    vmBanner();

    /* whether the program is being preinitialized (t3GetVMPreinitMode) */
    inPreinit();

    /* talk to the debugger (t3DebugTrace): see DebugTraceCheck and the
     *   rest */
    debugTrace(mode, ...);

    /* the program's global symbols or its macros, if it was compiled
     *   with them (t3GetGlobalSymbols): see GlobalSymbolTable */
    globalSymbols(which?);

    /* a new property, never used before (t3AllocProp) */
    newProperty();

    /* the stack, or one level of it (t3GetStackTrace): see
     *   StackTraceLocals */
    stackTrace(level?, flags?);

    /* a named argument of the caller (t3GetNamedArg) */
    namedArg(name, defval?);

    /* the names of the named arguments in effect (t3GetNamedArgList) */
    namedArgList();
}

/* debugTrace modes */
#define DebugTraceCheck     1
#define DebugTraceBreak     2
#define DebugTraceLog       3

/* setSay values that remove the function or the method */
#define SetSayNoFunc        1
#define SetSayNoMethod      2

/* which table globalSymbols gives */
#define GlobalSymbolTable   1
#define MacroTable          2

/* stackTrace flags, to be combined with | */
#define StackTraceLocals    0x0001
#define StackTraceDesc      0x0002

/* flags of a macro in the macro table */
#define MacroHasArgs        0x0001
#define MacroHasVarargs     0x0002

/*
 *   Input and output (tadsio.h), with tadsSay as print.
 */
intrinsic 'tads-io/030007'
{
    print(val, ...);
    setLogFile(fname, logType?);
    clearScreen();
    morePrompt();
    inputLine();
    inputKey();
    inputEvent(timeout?);
    inputDialog(icon, prompt, buttons, defaultButton, cancelButton);
    inputFile(prompt, dialogType, fileType, flags);
    timeDelay(delayMilliseconds);
    systemInfo(infoType, ...);
    statusMode(mode);
    statusRight(txt);
    resExists(fname);
    setScriptFile(filename, flags?);
    getLocalCharSet(which);
    flushOutput();
    inputLineTimeout(timeout?);
    inputLineCancel(reset);
    bannerCreate(parent, where, other, windowType, align, size, sizeUnits,
                 styleFlags);
    bannerDelete(handle);
    bannerClear(handle);
    bannerSay(handle, ...);
    bannerFlush(handle);
    bannerSizeToContents(handle);
    bannerGoTo(handle, row, col);
    bannerSetTextColor(handle, fg, bg);
    bannerSetScreenColor(handle, color);
    bannerGetInfo(handle);
    bannerSetSize(handle, size, sizeUnits, isAdvisory);
    logConsoleCreate(filename, charset, width);
    logConsoleClose(handle);
    logConsoleSay(handle, ...);
    logInputEvent(evt);
}
