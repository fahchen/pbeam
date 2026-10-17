/* Layout facts of the PCRE2 serialized form. pcre2_serialize_decode reads
   these fields without a length, so pbeam checks the encoded binary with
   the same layout before the call. */

#include <string.h>

#include "pcre2_internal.h"

/* The checks of pcre2_serialize_decode on the first code block
   (pcre2_serialize.c:210-217), and the range of `code_start`. The call
   frees the code block but not the character tables when one check fails,
   so pbeam checks them before the call. A `code_start` outside the block
   makes the matcher read outside it (pcre2_match.c:8018). */
int pbeam_pcre2_serialized_code_is_valid(const uint8_t *bytes, size_t blocksize)
{
  pcre2_real_code code;

  memcpy(&code, bytes, sizeof code);
  return code.magic_number == MAGIC_NUMBER
      && code.name_entry_size <= MAX_NAME_SIZE + IMM2_SIZE + 1
      && code.name_count <= MAX_NAME_COUNT
      && code.code_start >= sizeof code
      && code.code_start < blocksize;
}

/* Copy the name table of a compiled pattern into a caller buffer, so that
   the caller reads the entries in safe code. PCRE2 keeps the table in the
   code block, and pcre2_pattern_info gives its address. The function returns
   -1 when a query or the size check fails. */
int pbeam_pcre2_name_table(const void *code_ptr, uint8_t *buffer, size_t size,
                           uint32_t *count, uint32_t *entry_size)
{
  pcre2_code *code = (pcre2_code *)code_ptr;
  uint32_t number;
  uint32_t entry;
  unsigned char *table;

  if (pcre2_pattern_info(code, PCRE2_INFO_NAMECOUNT, &number) != 0) {
    return -1;
  }
  if (number == 0) {
    *count = 0;
    *entry_size = 0;
    return 0;
  }
  if (pcre2_pattern_info(code, PCRE2_INFO_NAMEENTRYSIZE, &entry) != 0) {
    return -1;
  }
  if (pcre2_pattern_info(code, PCRE2_INFO_NAMETABLE, &table) != 0) {
    return -1;
  }
  if (size < (size_t)number * (size_t)entry) {
    return -1;
  }
  memcpy(buffer, table, (size_t)number * (size_t)entry);
  *count = number;
  *entry_size = entry;
  return 0;
}

size_t pbeam_pcre2_serialized_code_start_offset(void)
{
  return offsetof(pcre2_real_code, code_start);
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
