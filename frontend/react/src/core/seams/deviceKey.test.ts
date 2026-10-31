import { IDBFactory } from "fake-indexeddb";
import { describe, expect, it } from "vitest";
import { hex, loadCoreForTests, toHex } from "../testing";
import { forgetDeviceSecret, hasWebCryptoX25519, newDeviceSecret, rememberedDeviceSecret, softwareDeviceKey } from "./deviceKey";

// RFC 7748, section 6.1: Alice and Bob.
const ALICE_SECRET = "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a";
const ALICE_PUBLIC = "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a";
const BOB_SECRET = "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb";
const BOB_PUBLIC = "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f";
const SHARED = "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742";

describe("the software device key", () => {
  it("is X25519: the RFC 7748 vector, both ways", async () => {
    const core = await loadCoreForTests();
    const alice = softwareDeviceKey(core, hex(ALICE_SECRET));
    const bob = softwareDeviceKey(core, hex(BOB_SECRET));
    expect(toHex(alice.publicKeyBytes())).toBe(ALICE_PUBLIC);
    expect(toHex(bob.publicKeyBytes())).toBe(BOB_PUBLIC);
    expect(toHex(alice.dhBytes(bob.publicKeyBytes()))).toBe(SHARED);
    expect(toHex(bob.dhBytes(alice.publicKeyBytes()))).toBe(SHARED);
    expect(() => softwareDeviceKey(core, hex("00"))).toThrow(/32 bytes/);
    expect(() => alice.dhBytes(hex("00"))).toThrow(/32 bytes/);
  });

  it("agrees with WebCrypto's X25519 on the same vector", async () => {
    // Node's WebCrypto has X25519, as the browsers do: the key the design
    // asks for exists; only the core's synchronous seam keeps it out.
    expect(await hasWebCryptoX25519()).toBe(true);
    const core = await loadCoreForTests();
    const bob = softwareDeviceKey(core, hex(BOB_SECRET));
    const alicePrivate = await crypto.subtle.importKey("pkcs8", pkcs8(hex(ALICE_SECRET)), { name: "X25519" }, false, ["deriveBits"]);
    const bobPublic = await crypto.subtle.importKey("raw", new Uint8Array(bob.publicKeyBytes()), { name: "X25519" }, false, []);
    const shared = new Uint8Array(await crypto.subtle.deriveBits({ name: "X25519", public: bobPublic }, alicePrivate, 256));
    expect(toHex(shared)).toBe(SHARED);
  });

  it("mints a 32-byte secret and remembers it in IndexedDB until forgotten", async () => {
    expect(newDeviceSecret()).toHaveLength(32);
    const factory = new IDBFactory();
    const first = await rememberedDeviceSecret(factory);
    expect(first).toHaveLength(32);
    expect(await rememberedDeviceSecret(factory)).toEqual(first);
    await forgetDeviceSecret(factory);
    expect(await rememberedDeviceSecret(factory)).not.toEqual(first);
  });
});

/** A raw X25519 private key as PKCS#8 (RFC 8410): the fixed prefix and the 32 bytes. */
function pkcs8(secret: Uint8Array): Uint8Array<ArrayBuffer> {
  const prefix = hex("302e020100300506032b656e04220420");
  const out = new Uint8Array(prefix.length + secret.length);
  out.set(prefix);
  out.set(secret, prefix.length);
  return out;
}
