// SPDX-License-Identifier: MIT OR Apache-2.0

import { describe, expect, it } from "vitest";

import { createAppQueryClient } from "./queryClient";

describe("createAppQueryClient", () => {
  it("runs queries and mutations whatever the OS says about the network", () => {
    const options = createAppQueryClient().getDefaultOptions();

    expect(options.queries?.networkMode).toBe("always");
    expect(options.mutations?.networkMode).toBe("always");
  });
});
