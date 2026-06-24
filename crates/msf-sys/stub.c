/*
 * Backend seam for msf.
 *
 * msf is a Swift *frontend* (lex -> parse -> sema). The single symbol it needs
 * a host/backend to provide is `module_stub_find`: the lookup of a module's
 * public type names, used by `sema_import_module` to predeclare imported types
 * when no runtime vocabulary (.msfvocab) is attached.
 *
 * quick-swift drives msf with bare source (no vocab), so a null implementation
 * is sufficient for the walking skeleton: imports simply resolve to nothing.
 * `MODULE_STUBS` is declared `extern` in the header and must be defined here to
 * satisfy the linker even though the null `module_stub_find` never reads it.
 */
#include "semantic/module_stubs.h"

const ModuleStub MODULE_STUBS[MODULE_STUB_COUNT] = {0};

const ModuleStub *module_stub_find(const char *module_name) {
  (void)module_name;
  return 0;
}
