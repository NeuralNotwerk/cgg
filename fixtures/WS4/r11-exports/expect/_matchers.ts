import { assertInstanceOf } from "@std/assert/instance-of";
import { assertEquals } from "@std/assert/equals";

// `exports` maps name the files these sub-paths reach.
export function toBeInstanceOf(value: unknown, expected: unknown) {
  assertInstanceOf(value, expected);
  assertEquals(value, expected);
}
