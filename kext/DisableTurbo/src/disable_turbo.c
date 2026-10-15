/*
 * DisableTurbo.kext - turns Intel Turbo Boost off while loaded.
 *
 * Copyright (C) 2026 Conehard
 * SPDX-License-Identifier: GPL-3.0-only
 *
 * Sets IA32_MISC_ENABLE (MSR 0x1A0) bit 38 "Turbo Mode Disable" on every
 * logical processor, and restores the original state on unload.
 * Reference: Intel SDM vol. 4 (MSRs) and vol. 2 (CPUID leaf 06H).
 * The technique was first published in DisableTurboBoost.kext by
 * Adam Strzelecki (Copyright (c) 2012, MIT License).
 */

#include <mach/mach_types.h>
#include <libkern/libkern.h>

#define IA32_MISC_ENABLE        0x1A0u
#define MISC_TURBO_DISABLE      (UINT64_C(1) << 38)

#define CPUID_LEAF_THERMAL_PWR  6u
#define CPUID_TURBO_AVAILABLE   (1u << 1)

/* Exported to kexts but missing from the public headers. */
extern void mp_rendezvous_no_intrs(void (*action)(void *), void *arg);

static inline uint64_t msr_read(uint32_t reg)
{
    uint32_t low, high;
    __asm__ volatile("rdmsr" : "=a"(low), "=d"(high) : "c"(reg));
    return ((uint64_t)high << 32) | low;
}

static inline void msr_write(uint32_t reg, uint64_t val)
{
    __asm__ volatile("wrmsr" : : "c"(reg), "a"((uint32_t)val), "d"((uint32_t)(val >> 32)));
}

static inline void cpuid_query(uint32_t leaf, uint32_t out[4])
{
    __asm__ volatile("cpuid"
                     : "=a"(out[0]), "=b"(out[1]), "=c"(out[2]), "=d"(out[3])
                     : "a"(leaf), "c"(0u));
}

static boolean_t cpu_has_turbo(void)
{
    uint32_t r[4];

    cpuid_query(0, r);
    if (r[0] < CPUID_LEAF_THERMAL_PWR) {
        return FALSE;
    }
    cpuid_query(CPUID_LEAF_THERMAL_PWR, r);
    return (r[0] & CPUID_TURBO_AVAILABLE) != 0;
}

struct turbo_request {
    boolean_t turbo_off;
};

static void apply_request(void *opaque)
{
    const struct turbo_request *req = (const struct turbo_request *)opaque;
    uint64_t cur = msr_read(IA32_MISC_ENABLE);
    uint64_t want = req->turbo_off ? (cur | MISC_TURBO_DISABLE)
                                   : (cur & ~MISC_TURBO_DISABLE);

    if (want != cur) {
        msr_write(IA32_MISC_ENABLE, want);
    }
}

static boolean_t g_turbo_was_off = FALSE;

static void switch_turbo(boolean_t turbo_off)
{
    struct turbo_request req = { turbo_off };
    uint64_t before = msr_read(IA32_MISC_ENABLE);

    mp_rendezvous_no_intrs(apply_request, &req);

    printf("DisableTurbo: Turbo Boost %s (IA32_MISC_ENABLE %llx -> %llx)\n",
           turbo_off ? "off" : "on", (unsigned long long)before,
           (unsigned long long)msr_read(IA32_MISC_ENABLE));
}

kern_return_t DisableTurbo_start(kmod_info_t *ki, void *data);
kern_return_t DisableTurbo_stop(kmod_info_t *ki, void *data);

kern_return_t DisableTurbo_start(kmod_info_t *ki, void *data)
{
    (void)ki;
    (void)data;

    if (!cpu_has_turbo()) {
        printf("DisableTurbo: this CPU has no Turbo Boost, nothing to do\n");
        return KERN_FAILURE;
    }

    g_turbo_was_off = (msr_read(IA32_MISC_ENABLE) & MISC_TURBO_DISABLE) != 0;
    switch_turbo(TRUE);
    return KERN_SUCCESS;
}

kern_return_t DisableTurbo_stop(kmod_info_t *ki, void *data)
{
    (void)ki;
    (void)data;

    /* Restore the original state; never enable a Turbo that was already off. */
    switch_turbo(g_turbo_was_off);
    return KERN_SUCCESS;
}

KMOD_EXPLICIT_DECL(dev.intelmactuner.DisableTurbo, "1.1.0", DisableTurbo_start, DisableTurbo_stop)
__private_extern__ kmod_start_func_t *_realmain = DisableTurbo_start;
__private_extern__ kmod_stop_func_t *_antimain = DisableTurbo_stop;
__private_extern__ int _kext_apple_cc = __APPLE_CC__;
