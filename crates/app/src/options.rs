//! Startup seat options share one prefix parser and duplicate check.
use qa_core::sys_events::SeatId;

pub fn parse_seat(value: &str) -> Option<SeatId> {
    value.parse().ok().and_then(SeatId::new)
}

pub fn parse_seat_value<T>(
    value: &str,
    parse: impl FnOnce(&str) -> Option<T>,
) -> Result<(SeatId, T), &'static str> {
    let (seat, value) = value
        .split_once(':')
        .ok_or("seat option needs seat:value")?;
    let seat = parse_seat(seat).ok_or("seat outside 0..4")?;
    Ok((seat, parse(value).ok_or("invalid seat option value")?))
}
pub fn insert_seat_value<T>(
    slots: &mut [Option<T>; SeatId::COUNT],
    count: &mut usize,
    seat: SeatId,
    value: T,
) -> Result<(), &'static str> {
    let slot = &mut slots[seat.index()];
    if slot.is_some() {
        return Err("duplicate seat option");
    }
    *slot = Some(value);
    *count = (*count).max(seat.index() + 1);
    Ok(())
}
