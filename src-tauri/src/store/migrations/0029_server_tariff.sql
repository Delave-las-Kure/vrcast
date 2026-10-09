-- T712 (owner's decision: «поле тариф») — the hosting plan's speed going out, Mbit/s, as the
-- owner writes it on the server's card. The diagnosis and the Viewers screen weigh the load
-- against it instead of the network card's own speed.
--
-- NULL — not given, which is what every profile had before: then the load is weighed against
-- the network card, and the screen says so.
ALTER TABLE server_profiles ADD COLUMN tariff_mbit INTEGER;
