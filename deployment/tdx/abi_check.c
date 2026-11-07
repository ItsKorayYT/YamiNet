#include <stdint.h>
#include "tdx_attest.h"
_Static_assert(sizeof(tdx_report_data_t)==64,"REPORTDATA ABI");
_Static_assert(sizeof(tdx_uuid_t)==16,"UUID ABI");
_Static_assert(sizeof(tdx_attest_error_t)==4,"return code ABI");
typedef tdx_attest_error_t (*quote_fn)(const tdx_report_data_t*,const tdx_uuid_t*,uint32_t,tdx_uuid_t*,uint8_t**,uint32_t*,uint32_t);
typedef tdx_attest_error_t (*free_fn)(uint8_t*);
_Static_assert(__builtin_types_compatible_p(__typeof__(&tdx_att_get_quote),quote_fn),"quote function ABI");
_Static_assert(__builtin_types_compatible_p(__typeof__(&tdx_att_free_quote),free_fn),"free function ABI");
