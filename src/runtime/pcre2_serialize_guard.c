/* Layout facts of the PCRE2 serialized form. pcre2_serialize_decode reads
   these fields without a length, so pbeam checks the encoded binary with
   the same layout before the call. */

#include <string.h>

#include "pcre2_internal.h"

/* The checks of pcre2_serialize_decode on the first code block
   (pcre2_serialize.c:210-217). The call frees the code block but not the
   character tables when one fails, so pbeam checks them before the call. */
int pbeam_pcre2_serialized_code_is_valid(const uint8_t *bytes)
{
  pcre2_real_code code;

  memcpy(&code, bytes, sizeof code);
  return code.magic_number == MAGIC_NUMBER
      && code.name_entry_size <= MAX_NAME_SIZE + IMM2_SIZE + 1
      && code.name_count <= MAX_NAME_COUNT;
}

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
