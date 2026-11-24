#charset "us-ascii"
#pragma once

/* 
 *   This header defines the test function set ('hexicon-vm-test' to the
 *   virtual machine).  This function set is
 *   intended primarily for use by the developers of the VM itself, for
 *   testing purposes.  User could should not use this function set, as the
 *   functionality here is not generally useful to "real" programs and is
 *   subject to future incompatible changes.  
 */

/* 
 *   The virtual machine's test interface 
 */
builtin 'hexicon-vm-test/010000'
{
    vmTestObjId(obj);
    vmTestObjGcState(id);
    vmTestCharCode(c);
}

