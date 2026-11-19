/*
 *   tok_dump - list the tokens the reference compiler's tokenizer finds
 *   in a source file, one to a line, as the compiler proper would see
 *   them: after preprocessing, with keywords recognized and strings
 *   translated.
 *
 *   This is not part of the original test suite.  It is a driver of
 *   Hexicon's own, built against the reference sources, so that Hexicon's
 *   tokenizer has something to be compared with.  It is modeled on the
 *   suite's test_tok.cpp, and sets the tokenizer up the same way.
 *
 *   Each line of output is a token's type, as its number in the
 *   reference's tc_toktyp_t, a tab, and the token's text.  In the text, a
 *   backslash is doubled and a byte that is not printable ASCII is given
 *   as \xNN.  Messages from the compiler are written in among the tokens,
 *   as they come.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "os.h"
#include "t3std.h"
#include "tctok.h"
#include "resload.h"
#include "tcmain.h"
#include "tchostsi.h"
#include "tcglob.h"
#include "tcprs.h"
#include "vmerr.h"
#include "vmimage.h"
#include "vmrunsym.h"
#include "vmmeta.h"
#include "t3test.h"
#include "tct3drv.h"

static void show_text(const char *txt, size_t len)
{
    for ( ; len != 0 ; ++txt, --len)
    {
        unsigned char c = (unsigned char)*txt;
        if (c == '\\')
            printf("\\\\");
        else if (c >= 32 && c < 127)
            putchar(c);
        else
            printf("\\x%02x", c);
    }
}

int main(int argc, char **argv)
{
    CResLoader *res_loader;
    CTcHostIfc *hostifc;
    int curarg;
    char pathbuf[OSFNMAX];

    test_init();
    hostifc = new CTcHostIfcStdio();
    os_get_special_path(pathbuf, sizeof(pathbuf), argv[0], OS_GSP_T3_RES);
    res_loader = new CResLoader(pathbuf);
    CTcMain::init(hostifc, res_loader, "us-ascii");
    G_tok->set_test_report_mode(TRUE);
    G_tcmain->set_test_report_mode(TRUE);

    err_try
    {
        /* the same definitions test_tok makes */
        G_tok->add_define("_MSC_VER", "1100");
        G_tok->add_define("_WIN32", "1");
        G_tok->add_define("_M_IX86", "500");
        G_tok->add_define("__STDC__", "0");
        G_tok->add_define("_INTEGRAL_MAX_BITS", "64");
        G_tok->add_define("__cplusplus", "1");

        for (curarg = 1 ; curarg < argc ; ++curarg)
        {
            char *p = argv[curarg];
            if (*p != '-')
                break;
            if (*(p + 1) == 'I')
                G_tok->add_inc_path(*(p + 2) == '\0' ? argv[++curarg] : p + 2);
            else
            {
                curarg = argc;
                break;
            }
        }
        if (curarg + 1 != argc)
        {
            printf("usage: tok_dump [-Idir] <source-file>\n");
            exit(1);
        }
        if (G_tok->set_source(argv[curarg], argv[curarg]))
        {
            printf("unable to open source file\n");
            exit(1);
        }

        while (G_tok->next() != TOKT_EOF)
        {
            const CTcToken *tok = G_tok->getcur();
            printf("%d\t", (int)tok->gettyp());
            show_text(tok->get_text(), tok->get_text_len());
            printf("\n");
        }
    }
    err_catch(exc)
    {
        printf("exception caught: %d\n", exc->get_error_code());
    }
    err_end;

    CTcMain::terminate();
    delete res_loader;
    delete hostifc;
    return 0;
}

/* ------------------------------------------------------------------------ */
/*
 *   Stand-ins for the parts of the compiler and the VM that the tokenizer
 *   never reaches here but its neighbors refer to.  These are the ones
 *   test_tok.cpp supplies, for the same reason.
 */
void CVmRuntimeSymbols::add_sym(const char *, size_t, const vm_val_t *)
{
}

vm_meta_entry_t *CVmMetaTable::get_entry_by_id(const char *) const
{
    return 0;
}

CTcSymObj *CTcParser::add_gen_obj_stat(CTcSymObj *)
{
    return 0;
}

void CTcParser::add_gen_obj_prop_stat(
    CTcSymObj *, CTcSymProp *, const CTcConstVal *)
{
}

void CTcParser::set_source_text_group_mode(int)
{
}

class CTPNInlineObject *CTcPrsOpUnary::parse_inline_object(int)
{
    return 0;
}

CTPNAnonFunc *CTcPrsOpUnary::parse_anon_func(int, int)
{
    return 0;
}

void CTcPrsSymtab::add_to_global_symtab(CTcPrsSymtab *, CTcSymbol *)
{
}

int CTcParser::alloc_ctx_arr_idx()
{
    return 0;
}

void CTcParser::init_local_ctx()
{
}

void CTPNStmObjectBase::add_implicit_constructor()
{
}

void CTPNObjDef::fold_proplist(CTcPrsSymtab *)
{
}

void CTPNStmObjectBase::add_prop_entry(CTPNObjProp *, int)
{
}

void CTPNStmObjectBase::delete_property(CTcSymProp *)
{
}

int CTPNStmObjectBase::parse_nested_obj_prop(
    CTPNObjProp* &, int *, tcprs_term_info *, CTcToken const *, int)
{
    return FALSE;
}

void CTPNStmBase::add_debug_line_rec()
{
}

void CTPNStmBase::add_debug_line_rec(CTcTokFileDesc *, long)
{
}
