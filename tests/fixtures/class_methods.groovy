package shop.orders

import java.time.LocalDate

class Order {
    String id
    LocalDate placed
    List<Item> items = []

    BigDecimal total() {
        items.sum { it.price * it.quantity } ?: 0
    }

    boolean isLate(LocalDate today) {
        return placed.plusDays(7).isBefore(today)
    }

    static Order empty(String id) {
        new Order(id: id, placed: LocalDate.now())
    }
}
