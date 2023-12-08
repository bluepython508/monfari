CREATE TABLE accounts (
	id TEXT NOT NULL PRIMARY KEY,
	type TEXT NOT NULL,
	name TEXT NOT NULL,
	notes TEXT NOT NULL DEFAULT '',
	enabled INT NOT NULL DEFAULT TRUE
) STRICT;

CREATE TABLE transactions (
	id TEXT NOT NULL PRIMARY KEY,
	amount TEXT NOT NULL,
	type TEXT NOT NULL, -- Received, Paid, MovePhys, MoveVirt, Convert
	new_amount TEXT, -- Convert only
	external_party TEXT, -- src ordst for received and paid respectively
	acc_1 TEXT NOT NULL REFERENCES accounts (id), -- phys acc for {,_virt} types, src for moves
	acc_2 TEXT NOT NULL REFERENCES accounts (id), -- virt acc for {,_virt} types, dst for moves
	notes TEXT NOT NULL DEFAULT ''
) STRICT;

CREATE TABLE commands (
	id TEXT NOT NULL PRIMARY KEY,
	command TEXT NOT NULL
) STRICT;
