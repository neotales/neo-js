// Type fragment owned by neojs-os. The CLI composition includes it because the
// CLI links this crate. Declaring `neo:os` records compiled availability; it
// does not grant permission (none is needed) or admit the module through a
// runtime allowlist. types/neo-js.core.d.ts omits both this module and the
// `os` global.

/**
 * Host OS information. `platform` and `arch` identify the Rust compilation
 * target, not necessarily a separately detected host. Optional fields follow
 * the same platform-specific rules as the `neo:os` exports. This discloses
 * bounded, best-effort OS metadata without filesystem or registry permission;
 * it does not grant arbitrary filesystem or registry access or invoke a
 * subprocess. The object exists only when the `neojs-os` crate is linked and
 * `neo:os` is admitted by the runtime module allowlist. Global properties
 * remain writable; named module exports use the runtime's private snapshot.
 */
declare const os: {
  platform: string;
  arch: string;
  version: string | undefined;
  codename: string | undefined;
  variant: string | undefined;
};

declare module "neo:os" {
  /** OS name of the Rust compilation target (`std::env::consts::OS`). */
  export const platform: string;
  /** CPU architecture of the Rust compilation target (`std::env::consts::ARCH`). */
  export const arch: string;
  /**
   * Linux: distribution VERSION_ID from `/etc/os-release` (or
   * `/usr/lib/os-release` only if the former is absent), not the kernel version.
   * Windows: DisplayVersion, or `build <number>` from decimal CurrentBuildNumber.
   * The fixed registry probes use the process-default WOW64 view (no view
   * selector); a 32-bit process can see different values or find them unavailable.
   * macOS: product version from `kern.osproductversion`.
   * Undefined if unsupported or no usable value is available.
   */
  export const version: string | undefined;
  /**
   * Linux VERSION_CODENAME, falling back to UBUNTU_CODENAME when unavailable.
   * Undefined off Linux or when no usable value is available.
   */
  export const codename: string | undefined;
  /**
   * Linux VARIANT_ID, falling back to VARIANT when unavailable; Windows EditionID.
   * Undefined on other platforms or when no usable value is available.
   */
  export const variant: string | undefined;
}
