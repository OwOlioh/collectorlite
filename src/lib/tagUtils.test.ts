import { describe, expect, it } from "vitest";
import { splitTagNames } from "./tagUtils";

describe("splitTagNames", () => {
  it("supports commas, Chinese commas and newlines", () => {
    expect(splitTagNames("值得再看, 知识，\n科技")).toEqual([
      "值得再看",
      "知识",
      "科技"
    ]);
  });

  it("removes empty items and surrounding whitespace", () => {
    expect(splitTagNames("  a ,, b ")).toEqual(["a", "b"]);
  });
});
