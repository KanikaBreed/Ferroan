# Found while working: QR9

- 25 November 2026, QR9: The QR26 review commit uses TB89/D73, also reserved for unpushed Q84 in the QR9 brief. The manager should reconcile the two before publication. QR9 adds no TB or D number.
- 25 November 2026, QR9: A replacement CreateAgent removes the previous session and its seed before registering the new seed. If registration fails (for example, a seed path already belongs to another agent), the old circuit has already been disabled. A follow-up should test the occupied-path case and preserve the prior session if the new registration fails.
