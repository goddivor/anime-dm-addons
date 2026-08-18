/* The C ABI every anime-dm source addon exposes.
 *
 * An addon is a native dynamic library. The host loads it, checks the ABI
 * version, hands it the service table below, pushes the user settings, then
 * calls the entry points with JSON in and JSON out.
 *
 * Every string an entry point returns belongs to the addon and must be released
 * with adm_free: the two sides never share an allocator. Symmetrically, a string
 * the host returns through the table is released with free_string.
 *
 * Answers come wrapped in an envelope: {"ok": <payload>} or {"error": "..."}.
 */
#ifndef ADM_ADDON_H
#define ADM_ADDON_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define ADM_ABI_VERSION 1u

/* Services the host lends to an addon. An addon never opens a socket itself, so
 * every request inherits the host user agent, timeouts and logging. */
typedef struct AdmHost {
    uint32_t size;  /* sizeof(AdmHost), so the table may grow later on */
    void* ctx;      /* opaque host state, handed back with every call */

    /* GET url with the headers of a JSON object; returns the body, or NULL. */
    char* (*http_get)(void* ctx, const char* url, const char* headers_json);
    /* Releases a string this table returned. */
    void (*free_string)(void* ctx, char* text);
    /* Writes a diagnostic line to the host log. */
    void (*log)(void* ctx, const char* message);
} AdmHost;

/* Shared entry points, provided by the ABI layer itself. */
uint32_t adm_abi_version(void);
void adm_init(const AdmHost* host);
void adm_set_config(const char* config_json);
void adm_free(char* text);

/* Source entry points, provided by each addon.
 * The argument and payload shapes are those of the addon-api crate. */
char* adm_metadata(void);
char* adm_preferences(void);
char* adm_popular(const char* input_json);
char* adm_latest(const char* input_json);
char* adm_search(const char* input_json);
char* adm_anime_details(const char* input_json);
char* adm_episode_list(const char* input_json);
char* adm_hoster_list(const char* input_json);
char* adm_video_list(const char* input_json);

#ifdef __cplusplus
}
#endif

#endif /* ADM_ADDON_H */
