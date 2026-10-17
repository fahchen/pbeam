/* Layout facts of the PCRE2 serialized form. pcre2_serialize_decode reads
   these fields without a length, so pbeam checks the encoded binary with
   the same layout before the call. */

#include "pcre2_internal.h"

size_t pbeam_pcre2_serialized_header_size(void)
{
  return sizeof(pcre2_serialized_data);
}

size_t pbeam_pcre2_serialized_tables_size(void)
{
  return TABLES_LENGTH;
}

size_t pbeam_pcre2_serialized_blocksize_offset(void)
{
  return offsetof(pcre2_real_code, blocksize);
}

size_t pbeam_pcre2_serialized_code_size(void)
{
  return sizeof(pcre2_real_code);
}

size_t pbeam_pcre2_serialized_magic_offset(void)
{
  return offsetof(pcre2_real_code, magic_number);
}

size_t pbeam_pcre2_serialized_number_of_codes_offset(void)
{
  return offsetof(pcre2_serialized_data, number_of_codes);
}
