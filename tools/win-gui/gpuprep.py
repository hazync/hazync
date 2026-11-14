"""Say "preparing the graphics card" while a new card's first proof compiles its GPU code (Xesktop, 11-13).

The Windows prover carries native GPU code for three card generations only (windows-prove.yml GPU_ARCHS = 61 70 80:
GTX 10 to RTX 40 series). A newer card (compute capability 9 and up: RTX 50 series, RTX PRO 6000, H100...) has the
NVIDIA driver compile that code on the first proof on the machine, about ten minutes in which nothing moves. The
first person to try the installer read that silence as a hang. So for such a card, until the proof reports its first
piece, Home says what is happening. Once a card has reported a piece it is remembered (the driver keeps what it
compiled), and later runs do not say it again.
"""

# The highest card generation the Windows prover has native code for (sm_80 runs on 8.0, 8.6 and 8.9).
NATIVE_MAX_MAJOR = 8

HEADLINE = "Preparing the graphics card"
DETAIL = ("This card is newer than the prover's built-in GPU code, so on its first proof the NVIDIA driver compiles "
          "that code for it: up to about 10 minutes, with nothing moving. This happens once on this machine; after it, the "
          "proof's pieces start counting.")


def card_key(card):
    """A card is remembered by its name and driver: a new driver drops what the old one compiled."""
    return f"{card.get('name', '?')}|{card.get('driver', '?')}"


def to_prepare(cards, cuda, prepared):
    """The cards of this run that will compile their GPU code first: [card dicts]. Empty for a CPU run."""
    if not cuda:
        return []
    done = set(prepared or [])
    return [c for c in cards or [] if c and c.get("cc_major", 0) > NATIVE_MAX_MAJOR and card_key(c) not in done]
