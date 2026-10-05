-- This sentinel proves qualified migrations do not alter a same-named public table.
CREATE TABLE public.users(id integer PRIMARY KEY, sentinel text NOT NULL);
INSERT INTO public.users VALUES (999, 'must-survive');
CREATE SCHEMA legacy;
