/**
 * T107 — a check that the GPL obligation is kept in fact.
 *
 * What is checked is not that there is a handsome screen, but two promises: that the
 * licence the application is distributed under is named, and that the address where the
 * source of **this very version** can be had is given. A link to "the latest" does not keep
 * the obligation: a person was handed a particular build, and their right is to its source.
 */

import { screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderIn } from "../../../test-utils";

const mockAppVersions = vi.fn();

vi.mock("../../../shared/ipc", async () => {
  const actual = await vi.importActual<typeof import("../../../shared/ipc")>("../../../shared/ipc");
  return {
    ...actual,
    ipc: {
      appVersions: () => mockAppVersions(),
      // The update section sits on this screen, next to the version it is about. It asks
      // nothing of the network here: a build with no update settings is the quiet case.
      updateStanding: () =>
        Promise.resolve({ current: "0.1.0", installed_as: "unpackaged", configured: false }),
    },
  };
});

const { About } = await import("../About");

beforeEach(() => {
  vi.clearAllMocks();
  mockAppVersions.mockResolvedValue({ app: "0.1.0", server: null, schema: 5 });
});

describe("about the application", () => {
  it("names the licence", async () => {
    renderIn(<About />);
    expect(await screen.findByText(/GNU General Public License/)).toBeInTheDocument();
  });

  it("gives the source address as a link somebody can follow", async () => {
    renderIn(<About />);
    const link = await screen.findByRole("link", { name: /github\.com/ });
    expect(link).toHaveAttribute("href", expect.stringContaining("github.com"));
  });

  it("ties the source to the version in hand — in the link, not only in the sentence", async () => {
    // **T360.** The wording said "the source of this very version" and named the tag, while the
    // link went to the repository root: the default branch, which is somebody else's code with
    // our promise attached. Saying the tag and linking elsewhere is the obligation kept in
    // appearance only, so the tag is asserted where it decides where a person lands.
    renderIn(<About />);
    expect(await screen.findByText("v0.1.0")).toBeInTheDocument();
    const link = await screen.findByRole("link", { name: /github\.com/ });
    expect(link).toHaveAttribute("href", expect.stringContaining("/tree/v0.1.0"));
  });

  it("points the list of other people's work at this build's list too", async () => {
    // The same fault one file along: `main`'s list belongs to a build the person does not have.
    renderIn(<About />);
    const list = await screen.findByRole("link", { name: /Перечень|list/i });
    expect(list).toHaveAttribute("href", expect.stringContaining("/blob/v0.1.0/THIRD-PARTY.md"));
  });

  it("says where the list of other people's work is", async () => {
    renderIn(<About />);
    expect(await screen.findByText("THIRD-PARTY.md")).toBeInTheDocument();
  });

  it("does not go quiet about the licence when the version could not be learned", async () => {
    // The core may be out of reach, and the obligation does not go anywhere because of it.
    mockAppVersions.mockRejectedValue(new Error("the core is unreachable"));
    renderIn(<About />);
    expect(await screen.findByText(/GNU General Public License/)).toBeInTheDocument();
    const link = screen.getByRole("link", { name: /github\.com/ });
    expect(link).toBeInTheDocument();
    // With no version there is no tag to point at, so the root is all there is — and the
    // promise of "this very version" is not made either, rather than made and not kept.
    expect(link).toHaveAttribute("href", "https://github.com/Delave-las-Kure/vrcast");
  });
});
