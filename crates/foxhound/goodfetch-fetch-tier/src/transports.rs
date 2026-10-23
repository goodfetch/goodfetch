//! The HTTP transports (spec #579; ADRs 0060, 0127): reqwest in `standard`
//! and `identified` Sessions, each filling the driver's `send(hop)` and
//! translating its library's failures into `TransportFailure`. Work in
//! progress (#759): see the ticket's report.
