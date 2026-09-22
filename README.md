# autobricks-dns
Autobricks DNS is a lightweight split-DNS service for intranets and isolated networks.
It returns configured private addresses for explicitly registered internal host names,
taking precedence over normal DNS resolution for those names. Queries for every other
name are forwarded unchanged to an upstream DNS server, preserving normal DNS behavior
for public or otherwise unregistered names.

Clients must use Autobricks DNS as their DNS resolver for this local-record override to
apply. It does not modify a client's existing operating-system DNS configuration by
itself.
