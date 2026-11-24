#charset "us-ascii"
#pragma once

/*
 *   Hexicon's system header: it brings in the headers for the virtual
 *   machine's built-in functions and classes, so that a program need include
 *   only this one. Hexad's header includes it.
 */

/*
 *   The input and output functions come from a header a program can choose,
 *   with the function that displays text. To use others, define
 *   HEXICON_IO_HEADER as the other header's name and _hexicon_io_say as the
 *   function, before this header is included (or with -D).
 */
#ifndef HEXICON_IO_HEADER
#define HEXICON_IO_HEADER "io.h"
#endif
#ifndef _hexicon_io_say
#define _hexicon_io_say(str) print(str)
#endif


/* the virtual machine's own functions */
#include "vm.h"

/* the general functions: data types, conversions, and the like */
#include "general.h"

/* input and output */
#include HEXICON_IO_HEADER

/* the built-in classes */
#include "systype.h"

/* if we're building a network program, include the net headers */
#ifdef HEXICON_INCLUDE_NET
#include <net.h>
#include <httpsrv.h>
#include <httpreq.h>
#endif


/* ------------------------------------------------------------------------ */
/*
 *   Break out of a callback iteration, such as a forEachInstance() loop.
 *   This can be used within the callback code to break out of the loop.  
 */
#define breakLoop throw BreakLoopSignal


/* ------------------------------------------------------------------------ */
/*
 *   Define a property value using an expression that's evaluated once per
 *   instance of the class where the property is defined.  This is used like
 *   so:
 *   
 *   class MyClass: MySuperClass
 *.    prop1 = perInstance(new SubObject())
 *.  ;
 *   
 *   Now, for each instance of MyClass, prop1 will be set to a separate
 *   instance of SubObject.
 *   
 *   Note that the per-instance property value is set "on demand" in each
 *   instance.  This means that a particular instance's copy of the property
 *   will be set when the property is first evaluated.  Note in particular
 *   that the value won't necessary be computed at compile time or during
 *   pre-initialization, because the value for a particular instance won't be
 *   calculated until the property is first used for a that instance.  
 */
#define perInstance(expr) (self.(targetprop) = (expr))


