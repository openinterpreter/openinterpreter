import io

from ...utils.lazy_import import lazy_import

# Lazy import of optional packages
np = lazy_import("numpy")
try:
    cv2 = lazy_import("cv2")
except Exception:
    cv2 = None  # Fixes colab error
PIL = lazy_import("PIL")
pytesseract = lazy_import("pytesseract")


def pytesseract_get_text(img):
    # List the attributes of pytesseract, which will trigger lazy loading of it
    attributes = dir(pytesseract)
    if pytesseract == None:
        raise ImportError("The pytesseract module could not be imported.")

    result = pytesseract.image_to_string(img)
    return result


def pytesseract_get_text_bounding_boxes(img):
    # Convert PIL Image to NumPy array
    img_array = np.array(img)

    # Convert the image to grayscale
    gray = cv2.cvtColor(img_array, cv2.COLOR_BGR2GRAY)

    # Use pytesseract to get the data from the image
    d = pytesseract.image_to_data(gray, output_type=pytesseract.Output.DICT)

    # Create an empty list to hold dictionaries for each bounding box
    boxes = []

    # Iterate through the number of detected boxes based on the length of one of the property lists
    for i in range(len(d["text"])):
        # For each box, create a dictionary with the properties you're interested in
        box = {
            "text": d["text"][i],
            "top": d["top"][i],
            "left": d["left"][i],
            "width": d["width"][i],
            "height": d["height"][i],
        }
        # Append this box dictionary to the list
        boxes.append(box)

    return boxes


def find_text_in_image(img, text, debug=False):
    # Convert PIL Image to NumPy array
    img_array = np.array(img)

    # Convert the image to grayscale
    gray = cv2.cvtColor(img_array, cv2.COLOR_BGR2GRAY)

    # Use pytesseract to get the data from the image
    d = pytesseract.image_to_data(gray, output_type=pytesseract.Output.DICT)

    # Initialize an empty list to store the centers of the bounding boxes
    centers = []

    # Get the number of detected boxes
    n_boxes = len(d["level"])

    # Create a copy of the grayscale image to draw on
    img_draw = np.array(gray.copy())

    # Convert the img_draw grayscale image to RGB
    img_draw = cv2.cvtColor(img_draw, cv2.COLOR_GRAY2RGB)

    id = 0

    # Loop through each box
    for i in range(n_boxes):
        if debug:
            # (DEBUGGING) Draw each box on the grayscale image
            cv2.rectangle(
                img_draw,
                (d["left"][i], d["top"][i]),
                (d["left"][i] + d["width"][i], d["top"][i] + d["height"][i]),
                (0, 255, 0),
                2,
            )
            # Draw the detected text in the rectangle in small font
            font = cv2.FONT_HERSHEY_SIMPLEX
            font_scale = 0.5
            font_color = (0, 0, 255)
            line_type = 2

            cv2.putText(
                img_draw,
                d["text"][i],
                (d["left"][i], d["top"][i] - 10),
                font,
                font_scale,
                font_color,
                line_type,
            )

        # Print the text of the box
        # If the text in the box matches the given text
        if text.lower() in d["text"][i].lower():
            # Find the start index of the matching text in the box
            start_index = d["text"][i].lower().find(text.lower())
            # Calculate the percentage of the box width that the start of the matching text represents
            start_percentage = start_index / len(d["text"][i])
            # Move the left edge of the box to the right by this percentage of the box width
            d["left"][i] = d["left"][i] + int(d["width"][i] * start_percentage)

            # Calculate the width of the matching text relative to the entire text in the box
            text_width_percentage = len(text) / len(d["text"][i])
            # Adjust the width of the box to match the width of the matching text
            d["width"][i] = int(d["width"][i] * text_width_percentage)

            # Calculate the center of the bounding box
            center = (
                d["left"][i] + d["width"][i] / 2,
                d["top"][i] + d["height"][i] / 2,
            )

            # Add the center to the list
            centers.append(center)

            # Draw the bounding box on the image in red and make it slightly larger
            larger = 10
            cv2.rectangle(
                img_draw,
                (d["left"][i] - larger, d["top"][i] - larger),
                (
                    d["left"][i] + d["width"][i] + larger,
                    d["top"][i] + d["height"][i] + larger,
                ),
                (255, 0, 0),
                7,
            )

            # Create a small black square background for the ID
            cv2.rectangle(
                img_draw,
                (
                    d["left"][i] + d["width"][i] // 2 - larger * 2,
                    d["top"][i] + d["height"][i] // 2 - larger * 2,
                ),
                (
                    d["left"][i] + d["width"][i] // 2 + larger * 2,
                    d["top"][i] + d["height"][i] // 2 + larger * 2,
                ),
                (0, 0, 0),
                -1,
            )

            # Put the ID in the center of the bounding box in red
            cv2.putText(
                img_draw,
                str(id),
                (
                    d["left"][i] + d["width"][i] // 2 - larger,
                    d["top"][i] + d["height"][i] // 2 + larger,
                ),
                cv2.FONT_HERSHEY_DUPLEX,
                1,
                (255, 155, 155),
                4,
            )

            # Increment id
            id += 1

    if not centers:
        word_centers = []
        for word in text.split():
            for i in range(n_boxes):
                if word.lower() in d["text"][i].lower():
                    center = (
                        d["left"][i] + d["width"][i] / 2,
                        d["top"][i] + d["height"][i] / 2,
                    )
                    center = (center[0] / 2, center[1] / 2)
                    word_centers.append(center)

        for center1 in word_centers:
            for center2 in word_centers:
                if (
                    center1 != center2
                    and (
                        (center1[0] - center2[0]) ** 2 + (center1[1] - center2[1]) ** 2
                    )
                    ** 0.5
                    <= 400
                ):
                    centers.append(
                        ((center1[0] + center2[0]) / 2, (center1[1] + center2[1]) / 2)
                    )
                    break
            if centers:
                break

    bounding_box_image = PIL.Image.fromarray(img_draw)
    bounding_box_image.format = img.format

    # Convert centers to relative
    img_width, img_height = img.size
    centers = [(x / img_width, y / img_height) for x, y in centers]

    # Debug by showing bounding boxes:
    # bounding_box_image.show()

    return centers
