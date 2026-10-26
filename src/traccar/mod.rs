//! Delivery of observations to Traccar's OsmAnd protocol endpoint.

mod forwarder;
mod queue;

pub use forwarder::Forwarder;
pub use queue::Queue;

use jiff::Timestamp;

/// One report for a Traccar device, encoded as OsmAnd form parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    /// Traccar device unique ID.
    pub device_id: String,
    pub time: Timestamp,
    /// `None` for reports without a position; Traccar then attaches the
    /// attributes to the device's last known location.
    pub fix: Option<Fix>,
    /// Custom attributes stored with the Traccar position.
    pub attributes: Vec<(&'static str, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fix {
    pub latitude: f64,
    pub longitude: f64,
    /// Knots, Traccar's default OsmAnd speed unit.
    pub speed: Option<f64>,
    /// Degrees clockwise from north.
    pub course: Option<u16>,
    /// Metres.
    pub altitude: Option<f64>,
    /// Metres.
    pub accuracy: Option<f64>,
}

impl Observation {
    /// Encodes the observation as an `application/x-www-form-urlencoded`
    /// OsmAnd request body.
    pub fn to_form(&self) -> String {
        let mut form = form_urlencoded::Serializer::new(String::new());
        form.append_pair("id", &self.device_id);
        form.append_pair("timestamp", &self.time.as_millisecond().to_string());
        if let Some(fix) = &self.fix {
            form.append_pair("lat", &format!("{:.7}", fix.latitude));
            form.append_pair("lon", &format!("{:.7}", fix.longitude));
            if let Some(speed) = fix.speed {
                form.append_pair("speed", &format!("{speed:.1}"));
            }
            if let Some(course) = fix.course {
                form.append_pair("bearing", &course.to_string());
            }
            if let Some(altitude) = fix.altitude {
                form.append_pair("altitude", &format!("{altitude:.1}"));
            }
            if let Some(accuracy) = fix.accuracy {
                form.append_pair("accuracy", &format!("{accuracy:.0}"));
            }
        }
        for (name, value) in &self.attributes {
            form.append_pair(name, value);
        }
        form.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_fix_and_attributes() {
        let observation = Observation {
            device_id: "N0CALL-9".to_owned(),
            time: "2026-10-27T12:00:00Z".parse().unwrap(),
            fix: Some(Fix {
                latitude: 49.058_333_33,
                longitude: -72.029_166_67,
                speed: Some(36.2),
                course: Some(88),
                altitude: Some(376.123),
                accuracy: Some(92.6),
            }),
            attributes: vec![
                ("aprsSymbol", "/>".to_owned()),
                ("aprsComment", "Hi & 73 ünïcode".to_owned()),
            ],
        };
        assert_eq!(
            observation.to_form(),
            "id=N0CALL-9&timestamp=1793102400000&lat=49.0583333&lon=-72.0291667&speed=36.2\
             &bearing=88&altitude=376.1&accuracy=93&aprsSymbol=%2F%3E\
             &aprsComment=Hi+%26+73+%C3%BCn%C3%AFcode"
        );
    }

    #[test]
    fn positionless_report_has_no_coordinates() {
        let observation = Observation {
            device_id: "N0CALL".to_owned(),
            time: "2026-10-27T12:00:00Z".parse().unwrap(),
            fix: None,
            attributes: vec![("aprsStatus", "QRV".to_owned())],
        };
        assert_eq!(
            observation.to_form(),
            "id=N0CALL&timestamp=1793102400000&aprsStatus=QRV"
        );
    }
}
