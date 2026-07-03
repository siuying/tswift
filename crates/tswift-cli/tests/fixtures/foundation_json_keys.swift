import Foundation

struct Person: Codable {
    let firstName: String
    let lastName: String
    let phoneNumber: String
}

struct Media: Codable {
    let imageURL: String
    let videoURL: String
}

// 1. convertToSnakeCase: struct fields camelCase -> snake_case JSON keys
var enc1 = JSONEncoder()
enc1.keyEncodingStrategy = .convertToSnakeCase
let d1 = try enc1.encode(Person(firstName: "John", lastName: "Doe", phoneNumber: "555-1234"))
print(String(data: d1, encoding: .utf8)!)

// 2. convertFromSnakeCase: decode from hand-written snake_case JSON
var dec2 = JSONDecoder()
dec2.keyDecodingStrategy = .convertFromSnakeCase
let json2 = "{\"first_name\":\"Jane\",\"last_name\":\"Smith\",\"phone_number\":\"555-5678\"}"
let person2 = try dec2.decode(Person.self, from: json2)
print(person2.firstName)
print(person2.lastName)
print(person2.phoneNumber)

// 3. snake_case round-trip: encode then decode reproduces original values
var encRT = JSONEncoder()
encRT.keyEncodingStrategy = .convertToSnakeCase
var decRT = JSONDecoder()
decRT.keyDecodingStrategy = .convertFromSnakeCase
let dataRT = try encRT.encode(Person(firstName: "Bob", lastName: "Lee", phoneNumber: "555-9999"))
let personRT = try decRT.decode(Person.self, from: dataRT)
print(personRT.firstName)
print(personRT.lastName)

// 4. Acronym case: imageURL -> image_url, videoURL -> video_url
let d4 = try enc1.encode(Media(imageURL: "https://example.com/img.jpg", videoURL: "https://example.com/vid.mp4"))
print(String(data: d4, encoding: .utf8)!)
